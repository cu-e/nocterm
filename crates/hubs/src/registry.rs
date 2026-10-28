//! Every hub this device knows, with the connection status every window shares.

use std::sync::Arc;

use gpui::{AppContext as _, Context, Entity, EventEmitter, Subscription, Task};
use nocterm_hubs_model::{
    AdminKeyError, HubAddress, HubColor, HubEndpointSpec, HubError, HubId, HubInfo, HubRecord,
    HubsSettings, PendingAdminKey,
};
use nocterm_store::SettingsHandle;

use crate::error::HubsError;
use crate::installed::Deps;
use crate::session_lists::{HubSessionLists, HubSessionListsEvent};
use crate::sign_in::{SignInEvent, SignInSession};
use crate::status::{HubStatus, Settled, retry_delay, settle_connect, settle_failure};

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
    lists: Option<LiveLists>,
}

struct LiveLists {
    entity: Entity<HubSessionLists>,
    _events: Subscription,
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
                        lists: None,
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
        let online = entry.status.is_online();
        let window = entry.lists.as_ref().map(|live| live.entity.clone());
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
        if online && let Some(window) = window {
            window.update(cx, |window, cx| window.refresh(cx));
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

    /// The shared lists of a hub's sessions, created on first use. `None` for an unknown
    /// hub.
    pub fn session_lists(
        &mut self,
        hub: &HubId,
        cx: &mut Context<Self>,
    ) -> Option<Entity<HubSessionLists>> {
        let index = self.entries.iter().position(|e| e.record.id() == *hub)?;
        if let Some(live) = &self.entries[index].lists {
            return Some(live.entity.clone());
        }
        let sessions = std::sync::Arc::clone(&self.deps.sessions);
        let entity = cx.new(|_| HubSessionLists::new(hub.clone(), sessions));
        let id = hub.clone();
        let events = cx.subscribe(&entity, move |hubs, _, event, cx| {
            if let HubSessionListsEvent::Failed(error) = event {
                hubs.lists_failed(&id, error, cx);
            }
        });
        let entry = &mut self.entries[index];
        entry.lists = Some(LiveLists {
            entity: entity.clone(),
            _events: events,
        });
        if entry.status.is_online() {
            entity.update(cx, |window, cx| window.refresh(cx));
        }
        Some(entity)
    }

    /// A call of the session lists failed. Only a hub that was online changes its status; a lost
    /// session sends the hub through a new connection, which finds out why.
    fn lists_failed(&mut self, hub: &HubId, error: &HubError, cx: &mut Context<Self>) {
        if !self.status(hub).is_some_and(|status| status.is_online()) {
            return;
        }
        match error {
            HubError::NotSignedIn => self.connect(hub, cx),
            other => {
                if let Some(settled) = settle_failure(other) {
                    self.rest(hub, settled, cx);
                }
            }
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
    pub fn pending_admin_keys(&self, hub: &HubId) -> Vec<PendingAdminKey> {
        if self.entry(hub).is_none() {
            return Vec::new();
        }
        self.deps.admin_keys.pending_admin_keys(hub)
    }

    /// Confirms the key of the administrator `account_id` of `hub`, shown with `fingerprint`,
    /// once a person compared it out of band. The account reads the groups it unlocks again.
    pub fn confirm_admin_key(
        &mut self,
        hub: &HubId,
        account_id: &str,
        fingerprint: &str,
        cx: &mut Context<Self>,
    ) -> Result<(), AdminKeyError> {
        let confirmed = self
            .deps
            .admin_keys
            .confirm_admin_key(hub, account_id, fingerprint);
        cx.notify();
        confirmed
    }

    /// Starts issuing a new recovery key for `hub`, which replaces the unconfirmed one. The
    /// dialog shows it once; `None` when `hub` is not listed.
    pub fn start_recovery_key_reissue(
        &mut self,
        hub: &HubId,
        cx: &mut Context<Self>,
    ) -> Option<Entity<SignInSession>> {
        self.entry(hub)?;
        let (deps, settings) = (self.deps.clone(), self.settings.clone());
        let session = cx.new(|cx| SignInSession::reissue(&deps, settings, hub, cx));
        // Saving the new key ends the offer: whoever shows it is told to look again.
        cx.subscribe(&session, |_, _, event, cx| {
            if matches!(event, SignInEvent::Finished(_)) {
                cx.notify();
            }
        })
        .detach();
        Some(session)
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

    /// The server whose account keeps this device's profiles and vault in step with its other
    /// devices, if one is chosen.
    pub fn account_hub(&self, cx: &gpui::App) -> Option<HubId> {
        self.settings.setting::<HubsSettings>(cx).account_hub()
    }

    /// Chooses the server whose account keeps profiles and vault, or none, in one edit of the
    /// settings. Choosing only records the choice: whether the account's keys are open is up to
    /// its sign-in.
    pub fn set_account(
        &mut self,
        hub: Option<&HubId>,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), HubsError>> {
        if let Some(hub) = hub
            && self.entry(hub).is_none()
        {
            return Task::ready(Err(HubsError::Unknown(hub.clone())));
        }
        let chosen = hub.cloned();
        let saving = self
            .settings
            .update_checked::<HubsSettings>(cx, move |settings| {
                settings.set_account_hub(chosen.as_ref());
                Ok(())
            });
        cx.spawn(async move |_, _| saving.await.map(|_| ()).map_err(HubsError::Save))
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
