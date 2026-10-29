//! Captured local-history identity; deliberately no stable Cloud authority.
use super::{OpenKeys, Slot};
use crate::classify::remote;
use nocterm_hubs_model::{
    HubError, HubId,
    source_identity::{
        Fingerprint, LocalAccountPartition, LocalAccountSource, LocalAccountSourceError,
    },
};
use std::sync::{Arc, Weak};

mod membership;
pub(crate) use membership::CapturedMembership;

/// Actual account/device metadata of one installed pool membership. Its Arc
/// identity ends at eviction or a new login, independently from token rotation.
#[derive(PartialEq, Eq)]
pub(crate) struct AccountSubject {
    account: String,
    device: String,
}
impl AccountSubject {
    pub(crate) fn new(account: String, device: String) -> Self {
        Self { account, device }
    }
    pub(crate) fn device(&self) -> &str {
        &self.device
    }
    pub(crate) fn account(&self) -> &str {
        &self.account
    }
}

/// Synchronous, non-I/O metadata access to the actual installed client. No device
/// keys, tokens or mutable SDK client cross this consumer-specific boundary.
pub(crate) trait AccountSubjects: Send + Sync {
    fn current(&self, hub: &HubId) -> Option<Arc<AccountSubject>>;
    fn with_current(
        &self,
        hub: &HubId,
        expected: &Arc<AccountSubject>,
        action: &mut dyn FnMut() -> Result<(), HubError>,
    ) -> Result<(), HubError>;
}

#[derive(Clone)]
pub(crate) struct LocalSourceProvider {
    slot: Arc<Slot>,
    subjects: Arc<dyn AccountSubjects>,
}
impl LocalSourceProvider {
    pub(crate) fn new(slot: Arc<Slot>, subjects: Arc<dyn AccountSubjects>) -> Self {
        Self { slot, subjects }
    }
    pub(crate) fn capture(&self) -> Result<Arc<dyn LocalAccountSource>, HubError> {
        self.capture_native()
            .map(|source| source as Arc<dyn LocalAccountSource>)
    }
    pub(crate) fn membership(&self) -> Result<Arc<CapturedMembership>, HubError> {
        CapturedMembership::capture(&self.slot, &self.subjects)
    }
    pub(crate) fn capture_native(&self) -> Result<Arc<CapturedAccount>, HubError> {
        self.membership()?.opened()
    }
}

pub(crate) struct CapturedAccount {
    slot: Weak<Slot>,
    subjects: Weak<dyn AccountSubjects>,
    partition: LocalAccountPartition,
    keys: Weak<OpenKeys>,
    subject: Weak<AccountSubject>,
}
impl CapturedAccount {
    /// The callback must not call Slot or Pool; both native ownership locks are held.
    pub(crate) fn publish<T>(&self, publish: impl FnOnce() -> T) -> Result<T, HubError> {
        let membership = self.membership()?;
        let expected = self.keys.upgrade().ok_or(HubError::Stale)?;
        membership.with_current(|| self.slot()?.while_keys_current(&expected, publish))
    }

    pub(crate) fn membership(&self) -> Result<Arc<CapturedMembership>, HubError> {
        let slot = self.slot()?;
        let subjects = self.subjects.upgrade().ok_or(HubError::Stale)?;
        let subject = self.subject()?;
        CapturedMembership::published(&slot, &subjects, &subject)
    }

    pub(crate) fn with_current<T>(
        &self,
        action: impl FnOnce(&Arc<OpenKeys>) -> Result<T, HubError>,
    ) -> Result<T, HubError> {
        let membership = self.membership()?;
        let expected = self.keys.upgrade().ok_or(HubError::Stale)?;
        membership.with_current(|| {
            let keys = self.slot()?.open_for(&membership.subject()?)?;
            if !Arc::ptr_eq(&keys, &expected) {
                return Err(HubError::Stale);
            }
            action(&keys)
        })
    }
    pub(crate) fn keys(&self) -> Result<Arc<OpenKeys>, HubError> {
        self.check_current().map_err(|_| HubError::NotSignedIn)?;
        self.keys.upgrade().ok_or(HubError::KeysLocked)
    }
    pub(crate) fn slot(&self) -> Result<Arc<Slot>, HubError> {
        self.slot.upgrade().ok_or(HubError::NotSignedIn)
    }
    pub(crate) fn subject(&self) -> Result<Arc<AccountSubject>, HubError> {
        self.check_current().map_err(|_| HubError::NotSignedIn)?;
        self.subject.upgrade().ok_or(HubError::NotSignedIn)
    }
}

impl LocalAccountSource for CapturedAccount {
    fn partition(&self) -> LocalAccountPartition {
        self.partition.clone()
    }
    fn check_current(&self) -> Result<(), LocalAccountSourceError> {
        let slot = self.slot.upgrade().ok_or(LocalAccountSourceError::Stale)?;
        let subjects = self
            .subjects
            .upgrade()
            .ok_or(LocalAccountSourceError::Stale)?;
        let expected_keys = self.keys.upgrade().ok_or(LocalAccountSourceError::Stale)?;
        let expected_subject = self
            .subject
            .upgrade()
            .ok_or(LocalAccountSourceError::Stale)?;
        let subject = subjects
            .current(slot.hub())
            .ok_or(LocalAccountSourceError::Stale)?;
        let keys = slot
            .open_for(&subject)
            .map_err(|_| LocalAccountSourceError::Stale)?;
        if !Arc::ptr_eq(&expected_keys, &keys)
            || !Arc::ptr_eq(&expected_subject, &subject)
            || keys.keyset().account_id() != subject.account()
        {
            return Err(LocalAccountSourceError::Stale);
        }
        Ok(())
    }
}

fn key_fingerprint(keys: &OpenKeys) -> Result<Fingerprint, HubError> {
    // The SDK already owns SHA-256 of this public key. Reuse the bounded hex
    // parser for its bytes; the certificate type does not establish hub trust here.
    let text = keys.keyset().public_key().fingerprint();
    if text.len() > 79 {
        return Err(remote(nocterm_hubs_model::FingerprintError::Length {
            found: text.len(),
        }));
    }
    let parsed = nocterm_hubs_model::Fingerprint::parse(&text.replace('-', "")).map_err(remote)?;
    Ok(Fingerprint::from_bytes(*parsed.as_bytes()))
}

#[cfg(test)]
mod tests;
