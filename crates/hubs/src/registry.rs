//! Every hub this device knows, with the connection status every window shares.

use std::sync::Arc;

use gpui::{AppContext as _, Context, Entity, EventEmitter, Subscription, Task};
use nocterm_hubs_model::{
    AccountKeyFuture, AdminKeyError, AdminKeyOffer, AgentAccessFuture, AgentAccessGrant, AgentCa,
    AgentTrustOverview, DeviceSummary, HostChoice, HubAddress, HubColor, HubEndpointSpec, HubError,
    HubFuture, HubId, HubInfo, HubRecord, HubsSettings, ServerTrust,
};
use nocterm_session::Secret;
use nocterm_store::SettingsHandle;

use crate::error::HubsError;
use crate::installed::Deps;
use crate::sign_in::{SignInEvent, SignInSession};
use crate::status::{HubStatus, Settled, retry_delay, settle_connect};

/// What changes in the registry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HubsEvent {
    /// A hub was added or removed, or a status changed.
    Changed,
}

/// A known hub as a menu shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HubSummary {
    pub id: HubId,
    pub label: String,
    pub address: HubAddress,
    pub color: Option<HubColor>,
    pub status: HubStatus,
}

struct HubEntry {
    record: HubRecord,
    status: HubStatus,
    /// Unsuccessful attempts to reach the hub in a row.
    failures: u32,
    /// The connection being made, or the wait before the next attempt.
    attempt: Option<Task<()>>,
}

/// The known hubs, in the order of the settings, and the status of each.
pub struct Hubs {
    settings: SettingsHandle,
    deps: Deps,
    entries: Vec<HubEntry>,
    _settings: Subscription,
    _keys: Task<()>,
}

impl EventEmitter<HubsEvent> for Hubs {}

impl Hubs {
    pub(crate) fn new(deps: Deps, settings: SettingsHandle, cx: &mut Context<Self>) -> Self {
        settings.register::<HubsSettings>(cx);
        let weak = cx.weak_entity();
        let observed = settings.observe(cx, move |cx| {
            weak.update(cx, |hubs, cx| hubs.sync(cx)).ok();
        });
        let keys = Self::redraw_on_key_changes(&deps, cx);
        let mut hubs = Self {
            settings,
            deps,
            entries: Vec::new(),
            _settings: observed,
            _keys: keys,
        };
        hubs.sync(cx);
        hubs
    }

    /// Tells the observers of the registry whenever account keys open or close, so a view of
    /// what depends on them (the offer of a new recovery key) follows a vault unlock at once.
    fn redraw_on_key_changes(deps: &Deps, cx: &mut Context<Self>) -> Task<()> {
        let sign_in = Arc::clone(&deps.sign_in);
        let mut seen = sign_in.keys_revision();
        cx.spawn(async move |this, cx| {
            loop {
                seen = sign_in.keys_changed(seen).await;
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        })
    }

    /// The known hubs in the order of the settings.
    pub fn list(&self) -> Vec<HubSummary> {
        self.entries
            .iter()
            .map(|entry| HubSummary {
                id: entry.record.id(),
                label: entry.record.display_label(),
                address: entry.record.address.clone(),
                color: entry.record.color,
                status: entry.status.clone(),
            })
            .collect()
    }

    pub fn summary(&self, hub: &HubId) -> Option<HubSummary> {
        self.list().into_iter().find(|summary| summary.id == *hub)
    }

    pub fn status(&self, hub: &HubId) -> Option<HubStatus> {
        self.entry(hub).map(|entry| entry.status.clone())
    }

    pub fn record(&self, hub: &HubId) -> Option<&HubRecord> {
        self.entry(hub).map(|entry| &entry.record)
    }

    fn entry(&self, hub: &HubId) -> Option<&HubEntry> {
        self.entries.iter().find(|entry| entry.record.id() == *hub)
    }

    fn entry_mut(&mut self, hub: &HubId) -> Option<&mut HubEntry> {
        self.entries
            .iter_mut()
            .find(|entry| entry.record.id() == *hub)
    }

    /// Brings the entries in line with the published settings: new servers start
    /// connecting, removed ones disappear, a changed trust connects again.
    fn sync(&mut self, cx: &mut Context<Self>) {
        let records = self.settings.setting::<HubsSettings>(cx).servers.clone();
        let mut previous = std::mem::take(&mut self.entries);
        let mut reconnect = Vec::new();
        for record in records {
            let id = record.id();
            let kept = previous.iter().position(|entry| entry.record.id() == id);
            let entry = match kept {
                Some(index) => {
                    let mut entry = previous.remove(index);
                    if entry.record.trust != record.trust {
                        reconnect.push(id);
                    }
                    entry.record = record;
                    entry
                }
                None => {
                    reconnect.push(id);
                    HubEntry {
                        record,
                        status: HubStatus::Connecting,
                        failures: 0,
                        attempt: None,
                    }
                }
            };
            self.entries.push(entry);
        }
        for hub in reconnect {
            self.connect(&hub, cx);
        }
        cx.emit(HubsEvent::Changed);
        cx.notify();
    }

    /// Connects to a hub with the trust the settings state.
    fn connect(&mut self, hub: &HubId, cx: &mut Context<Self>) {
        let Some(index) = self.entries.iter().position(|e| e.record.id() == *hub) else {
            return;
        };
        let entry = &mut self.entries[index];
        entry.status = HubStatus::Connecting;
        let connecting = self.deps.sign_in.connect(HubEndpointSpec {
            address: entry.record.address.clone(),
            trust: entry.record.trust,
        });
        let hub = hub.clone();
        entry.attempt = Some(cx.spawn(async move |this, cx| {
            let result = connecting.await;
            this.update(cx, |hubs, cx| hubs.settled(&hub, result, cx))
                .ok();
        }));
        cx.emit(HubsEvent::Changed);
        cx.notify();
    }

    fn settled(&mut self, hub: &HubId, result: Result<HubInfo, HubError>, cx: &mut Context<Self>) {
        let settled = settle_connect(&result);
        self.rest(hub, settled, cx);
    }

    /// Records where a hub ended up and, for a hub that did not answer, when to try again.
    fn rest(&mut self, hub: &HubId, settled: Settled, cx: &mut Context<Self>) {
        let now = cx.background_executor().now();
        let Some(entry) = self.entry_mut(hub) else {
            return;
        };
        entry.attempt = None;
        let mut retry = None;
        match settled {
            Settled::Status(status) => {
                entry.failures = 0;
                entry.status = status;
            }
            Settled::Retry => {
                entry.failures += 1;
                let delay = retry_delay(entry.failures);
                entry.status = HubStatus::Offline {
                    retry_at: now + delay,
                };
                retry = Some(delay);
            }
        }
        if let Some(delay) = retry {
            let again = hub.clone();
            let waiting = cx.spawn(async move |this, cx| {
                cx.background_executor().timer(delay).await;
                this.update(cx, |hubs, cx| hubs.connect(&again, cx)).ok();
            });
            if let Some(entry) = self.entry_mut(hub) {
                entry.attempt = Some(waiting);
            }
        }
        cx.emit(HubsEvent::Changed);
        cx.notify();
    }

    /// Tries a hub again now, instead of at the scheduled time.
    pub fn retry(&mut self, hub: &HubId, cx: &mut Context<Self>) {
        if self
            .status(hub)
            .is_some_and(|status| !matches!(status, HubStatus::Connecting | HubStatus::Online))
        {
            self.connect(hub, cx);
        }
    }

    /// Starts a sign-in to a server that is not known yet.
    pub fn start_sign_in(&mut self, cx: &mut Context<Self>) -> Entity<SignInSession> {
        let (deps, settings) = (self.deps.clone(), self.settings.clone());
        let session = cx.new(|_| SignInSession::new(&deps, settings));
        self.connect_when_signed_in(&session, cx);
        session
    }

    /// A finished sign-in connects the hub again, so a hub that was signed out or revoked
    /// comes back without a change of the settings. A new server is already connecting.
    fn connect_when_signed_in(&self, session: &Entity<SignInSession>, cx: &mut Context<Self>) {
        cx.subscribe(session, |hubs, _, event, cx| {
            if let SignInEvent::Finished(hub) = event
                && hubs.status(hub).is_some_and(|s| s != HubStatus::Connecting)
            {
                hubs.connect(hub, cx);
            }
        })
        .detach();
    }

    /// Starts signing in again to a known hub, with the trust its record states.
    pub fn start_sign_in_for(
        &mut self,
        hub: &HubId,
        cx: &mut Context<Self>,
    ) -> Option<Entity<SignInSession>> {
        let record = self.record(hub)?.clone();
        let (deps, settings) = (self.deps.clone(), self.settings.clone());
        let session = cx.new(|cx| SignInSession::resume(&deps, settings, record, cx));
        self.connect_when_signed_in(&session, cx);
        Some(session)
    }

    /// Whether the account keys of `hub` are open and the recovery key made with them was never
    /// confirmed as saved: a new one can be issued with [`Hubs::start_recovery_key_reissue`].
    pub fn recovery_key_unconfirmed(&self, hub: &HubId) -> bool {
        self.entry(hub).is_some() && self.deps.sign_in.recovery_key_unconfirmed(hub)
    }

    /// The administrators of `hub`'s organizations whose keys wait for a person to compare their
    /// fingerprints with the administrator's own; their groups stay out of the account until
    /// then. Empty while the account keys are closed. Observers of the registry are told when
    /// the list changes.
    pub fn pending_admin_keys(&self, hub: &HubId) -> Vec<AdminKeyOffer> {
        if self.entry(hub).is_none() {
            return Vec::new();
        }
        self.deps.admin_keys.pending_admin_keys(hub)
    }

    /// Confirms the key of the administrator `account_id` of `hub`, shown with `fingerprint`,
    /// once a person compared it out of band. The account reads the groups it unlocks again.
    pub fn confirm_admin_key(
        &mut self,
        offer: &AdminKeyOffer,
        fingerprint: &str,
        cx: &mut Context<Self>,
    ) -> Result<(), AdminKeyError> {
        if self.entry(offer.hub()).is_none() {
            return Err(AdminKeyError::Stale);
        }
        let confirmed = offer.confirm(fingerprint);
        cx.notify();
        confirmed
    }

    /// Captures the original recipient of a displayed recovery action.
    pub fn recovery_key_offer(
        &self,
        hub: &HubId,
    ) -> Result<nocterm_hubs_model::RecoveryKeyReissue, nocterm_hubs_model::HubError> {
        if self.entry(hub).is_none() {
            return Err(nocterm_hubs_model::HubError::Stale);
        }
        self.deps.sign_in.recovery_key_offer(hub)
    }

    /// Starts issuing a new recovery key for `hub`, which replaces the unconfirmed one. The
    /// dialog shows it once; `None` when `hub` is not listed.
    pub fn start_recovery_key_reissue(
        &mut self,
        hub: &HubId,
        offer: nocterm_hubs_model::RecoveryKeyReissue,
        cx: &mut Context<Self>,
    ) -> Option<Entity<SignInSession>> {
        self.entry(hub)?;
        if offer.hub() != hub || offer.check_current().is_err() {
            return None;
        }
        let (deps, settings) = (self.deps.clone(), self.settings.clone());
        let session = cx.new(|cx| SignInSession::reissue(&deps, settings, hub, offer, cx));
        // Saving the new key ends the offer: whoever shows it is told to look again.
        cx.subscribe(&session, |_, _, event, cx| {
            if matches!(event, SignInEvent::Finished(_)) {
                cx.notify();
            }
        })
        .detach();
        Some(session)
    }

    /// The devices of the account on `hub`. The call captures the original membership before it
    /// returns, so a result never belongs to an account that replaced it; a hub that is not
    /// listed is [`HubError::Stale`].
    pub fn list_devices(&self, hub: &HubId) -> HubFuture<Vec<DeviceSummary>> {
        if self.entry(hub).is_none() {
            return Box::pin(async { Err(HubError::Stale) });
        }
        self.deps.devices.list(hub)
    }

    /// Revokes `device` of the account on `hub`. Revoking this device is [`Hubs::sign_out`].
    pub fn revoke_device(&self, hub: &HubId, device: &str) -> HubFuture<()> {
        if self.entry(hub).is_none() {
            return Box::pin(async { Err(HubError::Stale) });
        }
        self.deps.devices.revoke(hub, device)
    }

    /// The servers with an open session, where the agent CA can be allowed now.
    pub fn agent_candidates(&self) -> Vec<HostChoice> {
        self.deps.agent_trust.connected_hosts()
    }

    /// The agent CA of the account on `hub` and the servers that trust it.
    pub fn agent_overview(&self, hub: &HubId) -> AgentAccessFuture<AgentTrustOverview> {
        self.deps.agent_trust.overview(hub)
    }

    /// Lets the agent CA sign in to the server of `connection`.
    pub fn allow_agent_host(&self, hub: &HubId, connection: &str) -> AgentAccessFuture<()> {
        self.deps.agent_trust.allow(hub, connection)
    }

    /// Stops trusting the server of `connection`; an unreachable one is removed later.
    pub fn deny_agent_host(&self, hub: &HubId, connection: &str) -> AgentAccessFuture<ServerTrust> {
        self.deps.agent_trust.deny(hub, connection)
    }

    /// Replaces the agent CA of the account on `hub`.
    pub fn reissue_agent_ca(&self, hub: &HubId) -> AgentAccessFuture<AgentCa> {
        self.deps.agent_trust.reissue_ca(hub)
    }

    /// The `authorized_keys` line of the agent CA.
    pub fn agent_ca_line(&self, hub: &HubId, connection: &str) -> AgentAccessFuture<String> {
        self.deps.agent_trust.ca_line(hub, connection)
    }

    /// The chat accesses agents hold on `hub`.
    pub fn agent_grants(&self, hub: &HubId) -> AgentAccessFuture<Vec<AgentAccessGrant>> {
        self.deps.agent_grants.list(hub)
    }

    /// Ends the chat access `grant_id` on `hub`.
    pub fn revoke_agent_grant(&self, hub: &HubId, grant_id: &str) -> AgentAccessFuture<()> {
        self.deps.agent_grants.revoke(hub, grant_id)
    }

    /// Whether the keys of the account on `hub` are open on this device.
    pub fn account_keys_open(&self, hub: &HubId) -> bool {
        self.deps.account_keys.is_open(hub)
    }

    /// Opens the keys of the account on `hub` with `password`.
    pub fn unlock_account_keys(&self, hub: &HubId, password: Secret) -> AccountKeyFuture<()> {
        self.deps.account_keys.unlock(hub, password)
    }

    /// Closes the keys of the account on `hub`.
    pub fn lock_account_keys(&self, hub: &HubId) {
        self.deps.account_keys.lock(hub);
    }

    /// The user connected to `connection`: every hub that is online compares the server's
    /// `authorized_keys` with its registry. A connection no registry knows is left alone.
    pub fn reconcile_agent_host(&self, connection: &str, cx: &mut Context<Self>) {
        for entry in &self.entries {
            if !matches!(entry.status, HubStatus::Online) {
                continue;
            }
            let reconciling = self
                .deps
                .agent_trust
                .reconcile(&entry.record.id(), connection);
            cx.spawn(async move |_, _| {
                let _ = reconciling.await;
            })
            .detach();
        }
    }

    /// Ends this device's session on the hub and erases its credentials. The server stays
    /// in the list, signed out.
    pub fn sign_out(&mut self, hub: &HubId, cx: &mut Context<Self>) -> Task<Result<(), HubsError>> {
        if self.entry(hub).is_none() {
            return Task::ready(Err(HubsError::Unknown(hub.clone())));
        }
        let signing_out = self.deps.sign_in.sign_out(hub);
        let hub = hub.clone();
        cx.spawn(async move |this, cx| {
            let result = signing_out.await;
            this.update(cx, |hubs, cx| {
                if result.is_ok() {
                    hubs.rest(&hub, Settled::Status(HubStatus::SignedOut), cx);
                }
            })
            .ok();
            result.map_err(HubsError::from)
        })
    }

    /// The settings the registry edits, for a text field that saves a server's name through
    /// them in the same one-edit way as [`Hubs::set_label`].
    pub fn settings(&self) -> SettingsHandle {
        self.settings.clone()
    }

    /// Renames a server in the settings, in one edit. An empty name shows the address.
    pub fn set_label(
        &mut self,
        hub: &HubId,
        label: &str,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), HubsError>> {
        let label = label.to_owned();
        self.edit_record(hub, cx, move |settings, hub| {
            settings.set_label(hub, &label);
        })
    }

    /// Marks a server with a colour, or clears the mark, in one edit of the settings.
    pub fn set_color(
        &mut self,
        hub: &HubId,
        color: Option<HubColor>,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), HubsError>> {
        self.edit_record(hub, cx, move |settings, hub| {
            settings.set_color(hub, color);
        })
    }

    fn edit_record(
        &mut self,
        hub: &HubId,
        cx: &mut Context<Self>,
        edit: impl FnOnce(&mut HubsSettings, &HubId) + 'static,
    ) -> Task<Result<(), HubsError>> {
        if self.entry(hub).is_none() {
            return Task::ready(Err(HubsError::Unknown(hub.clone())));
        }
        let id = hub.clone();
        let saving = self
            .settings
            .update_checked::<HubsSettings>(cx, move |settings| {
                edit(settings, &id);
                Ok(())
            });
        cx.spawn(async move |_, _| saving.await.map(|_| ()).map_err(HubsError::Save))
    }

    /// Removes a server from the settings and erases the credentials kept for it. When the
    /// settings cannot be saved the server stays listed without credentials, and the hub
    /// connects again to show where it stands.
    pub fn remove(&mut self, hub: &HubId, cx: &mut Context<Self>) -> Task<Result<(), HubsError>> {
        if self.entry(hub).is_none() {
            return Task::ready(Err(HubsError::Unknown(hub.clone())));
        }
        if let Err(error) = self.deps.sign_in.forget(hub) {
            return Task::ready(Err(error.into()));
        }
        let id = hub.clone();
        let removed = hub.clone();
        let saving = self
            .settings
            .update_checked::<HubsSettings>(cx, move |settings| {
                settings.remove(&removed);
                Ok(())
            });
        cx.spawn(async move |this, cx| {
            let saved = saving.await;
            if saved.is_err() {
                this.update(cx, |hubs, cx| hubs.connect(&id, cx)).ok();
            }
            saved.map(|_| ()).map_err(HubsError::Save)
        })
    }
}

#[cfg(test)]
mod tests;
