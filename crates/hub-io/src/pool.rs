//! The clients of the hubs this device is signed in to, and where it keeps their credentials.

pub(crate) mod captured;
pub(crate) mod observation;
use observation::{MembershipEnd, ObservedLogin};

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{
    Arc, Mutex, MutexGuard, PoisonError,
    atomic::{AtomicBool, Ordering},
};

use nocterm_hub_client::{DeviceInfo, HubClient};
use nocterm_hubs_model::{HubAddress, HubError, HubId, TrustPolicy};

use crate::account::source::{AccountSubject, AccountSubjects};
use crate::account::{Accounts, OpenKeys, Slot};
use crate::classify::credential_file;
use crate::credentials::{CredentialDir, CredentialFile, CredentialLoginId, LoginPublication};

/// The application name a hub lists beside the device.
const APP_NAME: &str = "Nocterm";

/// The client of a hub and the trust it was connected with.
struct Pooled {
    trust: TrustPolicy,
    client: HubClient,
    subject: Option<Arc<AccountSubject>>,
    login: Option<CredentialLoginId>,
}

/// The exact native owner created by one successful explicit publication.
pub(crate) struct PublishedLogin {
    login: CredentialLoginId,
    subject: Arc<AccountSubject>,
}
impl PublishedLogin {
    pub(crate) fn into_parts(self) -> (CredentialLoginId, Arc<AccountSubject>) {
        (self.login, self.subject)
    }
}

/// One client per hub: clients of a hub share the turn in which its tokens are renewed, and
/// two of them would renew the same refresh token against each other.
///
/// The pool also knows the account keys of the hubs, only to wipe them when a hub revokes this
/// device: whatever the hub no longer trusts leaves nothing of its account behind.
pub(crate) struct Pool {
    credentials: CredentialDir,
    device: DeviceInfo,
    accounts: Arc<Accounts>,
    clients: Mutex<HashMap<HubId, Pooled>>,
    stopped: AtomicBool,
}

impl Pool {
    pub(crate) fn new(
        credentials_dir: PathBuf,
        device_name: &str,
        accounts: Arc<Accounts>,
    ) -> Self {
        Self {
            credentials: CredentialDir::new(credentials_dir),
            device: DeviceInfo::new(device_name, APP_NAME),
            accounts,
            clients: Mutex::default(),
            stopped: AtomicBool::new(false),
        }
    }

    pub(crate) fn stop(&self) {
        self.stopped.store(true, Ordering::Release);
    }
    pub(crate) fn is_stopped(&self) -> bool {
        self.stopped.load(Ordering::Acquire)
    }
    pub(crate) fn device(&self) -> &DeviceInfo {
        &self.device
    }

    pub(crate) fn file(&self, address: &HubAddress) -> Result<CredentialFile, HubError> {
        self.credentials.file(address).map_err(credential_file)
    }

    fn clients(&self) -> MutexGuard<'_, HashMap<HubId, Pooled>> {
        self.clients.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Publishes only this captured file origin and original native selection.
    /// All paths acquire clients before the credential file lock.
    pub(crate) fn publish_login(
        &self,
        hub: HubId,
        trust: TrustPolicy,
        client: HubClient,
        publication: LoginPublication<'_>,
        expected: &ObservedLogin,
    ) -> Result<PublishedLogin, HubError> {
        let subject = Arc::new(subject_of(&client).ok_or(HubError::NotSignedIn)?);
        let session = publication.session();
        if subject.account() != session.account_id || subject.device() != session.device_id {
            return Err(HubError::Stale);
        }
        let slot = self.accounts.slot(&hub)?;
        let mut clients = self.clients();
        self.validate_observed(&clients, &hub, &slot, expected)?;
        let login = publication.save().map_err(credential_file)?;
        publication.bind(&client, login);
        slot.begin_membership(&subject);
        clients.insert(
            hub,
            Pooled {
                trust,
                client,
                subject: Some(subject.clone()),
                login: Some(login),
            },
        );
        Ok(PublishedLogin { login, subject })
    }

    /// A reconnect/refresh retains membership only for the same actual subject
    /// and trust. Explicit sign-in publishes a new local login even for the same ids.
    pub(crate) fn renew_observed(
        &self,
        hub: HubId,
        trust: TrustPolicy,
        client: HubClient,
        login: Option<CredentialLoginId>,
        expected: &ObservedLogin,
    ) -> Result<ObservedLogin, HubError> {
        self.renew_inner(hub, trust, client, login, Some(expected))
    }

    fn renew_inner(
        &self,
        hub: HubId,
        trust: TrustPolicy,
        client: HubClient,
        login: Option<CredentialLoginId>,
        expected: Option<&ObservedLogin>,
    ) -> Result<ObservedLogin, HubError> {
        let actual = subject_of(&client).ok_or(HubError::NotSignedIn)?;
        let slot = self.accounts.slot(&hub)?;
        let mut clients = self.clients();
        if let Some(expected) = expected {
            self.validate_observed(&clients, &hub, &slot, expected)?;
        }
        if !self
            .file(hub.address())?
            .matches_login(actual.account(), actual.device(), login)
            .map_err(credential_file)?
        {
            return Err(HubError::Stale);
        }
        let retained = clients.get(&hub).and_then(|pooled| {
            let subject = pooled.subject.as_ref()?;
            (pooled.trust == trust
                && pooled.login == login
                && pooled.client.info().hub_id == client.info().hub_id
                && &actual == subject.as_ref()
                && subject_of(&pooled.client).as_ref() == Some(subject.as_ref()))
            .then(|| Arc::clone(subject))
        });
        let subject = match retained {
            Some(subject) => subject,
            None => {
                let subject = Arc::new(actual);
                slot.begin_membership(&subject);
                subject
            }
        };
        clients.insert(
            hub,
            Pooled {
                trust,
                client,
                subject: Some(subject.clone()),
                login,
            },
        );
        Ok(ObservedLogin::new(Some(&subject), slot.observed()))
    }

    /// The client of the hub if it was connected with this trust.
    pub(crate) fn connected(
        &self,
        hub: &HubId,
        trust: TrustPolicy,
    ) -> Option<(HubClient, Option<CredentialLoginId>)> {
        let clients = self.clients();
        let pooled = clients.get(hub)?;
        (pooled.trust == trust).then(|| (pooled.client.clone(), pooled.login))
    }

    /// A direct configured-hub removal serializes against every pending login.
    pub(crate) fn forget_hub(&self, hub: &HubId) -> Result<(), HubError> {
        let slot = self.accounts.slot(hub)?;
        let mut clients = self.clients();
        clients.remove(hub);
        let cache = slot.erase();
        let file = self
            .file(hub.address())?
            .erase()
            .map_err(crate::classify::credentials);
        if cache.is_ok() {
            self.accounts.evict_if(hub, &slot);
        }
        cache.and(file)
    }

    /// Acquires only this original membership, synchronously and without I/O.
    pub(crate) fn captured(
        &self,
        hub: &HubId,
        expected: &Arc<AccountSubject>,
    ) -> Result<HubClient, HubError> {
        self.with_captured(hub, expected, |client| Ok(client.clone()))
    }

    /// One admission policy for original native I/O and synchronous publication.
    /// The action may lock Slot/cache, but must never await or call back into Pool.
    pub(crate) fn with_captured<T>(
        &self,
        hub: &HubId,
        expected: &Arc<AccountSubject>,
        action: impl FnOnce(&HubClient) -> Result<T, HubError>,
    ) -> Result<T, HubError> {
        let clients = self.clients();
        let pooled = self.matching(&clients, hub, expected)?;
        action(&pooled.client)
    }

    fn matching<'a>(
        &self,
        clients: &'a HashMap<HubId, Pooled>,
        hub: &HubId,
        expected: &Arc<AccountSubject>,
    ) -> Result<&'a Pooled, HubError> {
        let pooled = clients.get(hub).ok_or(HubError::Stale)?;
        let subject = pooled.subject.as_ref().ok_or(HubError::Stale)?;
        if self.is_stopped()
            || !Arc::ptr_eq(subject, expected)
            || subject_of(&pooled.client).as_ref() != Some(expected.as_ref())
        {
            return Err(HubError::Stale);
        }
        Ok(pooled)
    }

    pub(crate) fn observe(&self, hub: &HubId) -> Result<(Arc<Slot>, ObservedLogin), HubError> {
        // Use the existing subject reader to retire an SDK subject that already ended.
        // Otherwise a cleared SDK session would leave reconnect stuck on a stale map row.
        let _ = self.current(hub);
        let slot = self.accounts.slot(hub)?;
        let clients = self.clients();
        if self.is_stopped() {
            return Err(HubError::Stale);
        }
        let subject = clients.get(hub).and_then(|pooled| pooled.subject.as_ref());
        if let Some(subject) = subject {
            self.matching(&clients, hub, subject)?;
        }
        let observed = ObservedLogin::new(subject, slot.observed());
        Ok((slot, observed))
    }

    fn validate_observed(
        &self,
        clients: &HashMap<HubId, Pooled>,
        hub: &HubId,
        slot: &Slot,
        expected: &ObservedLogin,
    ) -> Result<(), HubError> {
        if self.is_stopped() || slot.hub() != hub {
            return Err(HubError::Stale);
        }
        match expected.subject()? {
            Some(subject) => {
                self.matching(clients, hub, &subject)?;
                return slot.belongs_to(&subject);
            }
            None if clients.contains_key(hub) => return Err(HubError::Stale),
            None => {}
        }
        slot.check_observed(expected.receipt())
    }

    pub(crate) fn with_observed<T>(
        &self,
        hub: &HubId,
        slot: &Slot,
        expected: &ObservedLogin,
        action: impl FnOnce() -> Result<T, HubError>,
    ) -> Result<T, HubError> {
        let clients = self.clients();
        self.validate_observed(&clients, hub, slot, expected)?;
        action()
    }

    pub(crate) fn observe_original(
        &self,
        hub: &HubId,
        expected: &Arc<AccountSubject>,
    ) -> Result<ObservedLogin, HubError> {
        let slot = self.accounts.slot(hub)?;
        self.with_captured(hub, expected, |_| {
            slot.belongs_to(expected)?;
            Ok(ObservedLogin::new(Some(expected), slot.observed()))
        })
    }

    pub(crate) fn retire_captured(
        &self,
        hub: &HubId,
        expected: &Arc<AccountSubject>,
        slot: &Slot,
        keys: &Arc<OpenKeys>,
    ) {
        if self.retire_membership(hub, expected)
            && let Err(error) = slot.erase_if(keys)
        {
            tracing::warn!(%error, "cannot erase captured revoked account cache");
        }
    }

    /// Retires the exact membership and its local file login; relay callers
    /// without unlocked keys use this same conditional credential boundary.
    pub(crate) fn retire_membership(&self, hub: &HubId, expected: &Arc<AccountSubject>) -> bool {
        match self.retire_original(hub, expected, false) {
            Ok(Some(ended)) => {
                let (_, result) = ended.into_parts();
                if let Err(error) = result {
                    tracing::warn!(%error,"cannot retire original revoked membership");
                }
                true
            }
            Ok(None) => false,
            Err(error) => {
                tracing::warn!(%error,"cannot retire original revoked membership");
                false
            }
        }
    }

    pub(crate) fn end_membership(
        &self,
        hub: &HubId,
        expected: &Arc<AccountSubject>,
        expire: bool,
    ) -> Result<MembershipEnd, HubError> {
        self.retire_original(hub, expected, expire)?
            .ok_or(HubError::Stale)
    }

    fn retire_original(
        &self,
        hub: &HubId,
        expected: &Arc<AccountSubject>,
        expire: bool,
    ) -> Result<Option<MembershipEnd>, HubError> {
        let mut clients = self.clients();
        if !clients
            .get(hub)
            .and_then(|pooled| pooled.subject.as_ref())
            .is_some_and(|subject| Arc::ptr_eq(subject, expected))
        {
            return Ok(None);
        }
        let slot = self.accounts.slot(hub)?;
        let login = clients.remove(hub).and_then(|pooled| pooled.login);
        let mut slot_error = None;
        let retired = self
            .file(hub.address())?
            .retire_if(
                expected.account(),
                expected.device(),
                login,
                !expire,
                || {
                    let result = if expire {
                        slot.close_for(expected).map(|_| ())
                    } else {
                        slot.erase_for(expected)
                    };
                    slot_error = result.err();
                },
            )
            .map_err(credential_file);
        if !matches!(&retired, Ok(true)) {
            // A newer private-file login belongs to somebody else. Drop only our
            // original in-memory keys; never remove that login's disk cache.
            let _ = slot.close_for(expected);
        }
        let result = retired.and_then(|owned| {
            if let Some(error) = slot_error {
                return Err(error);
            }
            if owned { Ok(()) } else { Err(HubError::Stale) }
        });
        Ok(Some(MembershipEnd::new(
            ObservedLogin::new(None, slot.observed()),
            result,
        )))
    }
}

impl AccountSubjects for Pool {
    fn with_current(
        &self,
        hub: &HubId,
        expected: &Arc<AccountSubject>,
        action: &mut dyn FnMut() -> Result<(), HubError>,
    ) -> Result<(), HubError> {
        self.with_captured(hub, expected, |_| action())
    }

    fn current(&self, hub: &HubId) -> Option<Arc<AccountSubject>> {
        let mut clients = self.clients();
        let pooled = clients.get(hub)?;
        let subject = pooled.subject.as_ref()?;
        if subject_of(&pooled.client).as_ref() == Some(subject.as_ref()) {
            return Some(Arc::clone(subject));
        }
        // A changed or cleared SDK subject cannot later revive this membership.
        let original = Arc::clone(subject);
        clients.remove(hub);
        if let Some(slot) = self.accounts.loaded_slot(hub) {
            let _ = slot.close_for(&original);
        }
        None
    }
}

pub(crate) fn subject_of(client: &HubClient) -> Option<AccountSubject> {
    let session = client.session()?;
    Some(AccountSubject::new(session.account_id, session.device_id))
}

#[cfg(test)]
mod tests;
