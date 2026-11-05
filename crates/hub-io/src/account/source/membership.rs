//! Native login membership remains usable for explicit unlock while keys are locked.
use super::{AccountSubject, AccountSubjects, CapturedAccount, key_fingerprint};
use crate::account::Slot;
use nocterm_hubs_model::source_identity::{
    LegacyConfiguredAccount, LocalAccountPartition, NamespaceId,
};
use nocterm_hubs_model::{HubError, HubId};
use std::sync::{Arc, Weak};

pub(crate) struct CapturedMembership {
    slot: Weak<Slot>,
    subjects: Weak<dyn AccountSubjects>,
    subject: Weak<AccountSubject>,
    hub: HubId,
}

impl CapturedMembership {
    pub(super) fn capture(
        slot: &Arc<Slot>,
        subjects: &Arc<dyn AccountSubjects>,
    ) -> Result<Arc<Self>, HubError> {
        let subject = subjects.current(slot.hub()).ok_or(HubError::NotSignedIn)?;
        Self::published(slot, subjects, &subject)
    }

    pub(crate) fn published(
        slot: &Arc<Slot>,
        subjects: &Arc<dyn AccountSubjects>,
        subject: &Arc<AccountSubject>,
    ) -> Result<Arc<Self>, HubError> {
        let captured = Arc::new(Self {
            slot: Arc::downgrade(slot),
            subjects: Arc::downgrade(subjects),
            subject: Arc::downgrade(subject),
            hub: slot.hub().clone(),
        });
        captured.check()?;
        Ok(captured)
    }

    pub(crate) fn hub(&self) -> &HubId {
        &self.hub
    }

    pub(crate) fn slot(&self) -> Result<Arc<Slot>, HubError> {
        self.slot.upgrade().ok_or(HubError::Stale)
    }

    pub(crate) fn subject(&self) -> Result<Arc<AccountSubject>, HubError> {
        self.subject.upgrade().ok_or(HubError::Stale)
    }

    pub(crate) fn check(&self) -> Result<(), HubError> {
        self.with_current(|| Ok(()))
    }

    pub(crate) fn opened(self: &Arc<Self>) -> Result<Arc<CapturedAccount>, HubError> {
        self.with_current(|| {
            let slot = self.slot()?;
            let subject = self.subject()?;
            let keys = slot.open_for(&subject)?;
            let account =
                NamespaceId::new(subject.account().to_owned()).map_err(crate::classify::remote)?;
            let partition = LocalAccountPartition::LegacyConfigured(LegacyConfiguredAccount::new(
                slot.hub().clone(),
                account,
                key_fingerprint(&keys)?,
            ));
            Ok(Arc::new(CapturedAccount {
                slot: Arc::downgrade(&slot),
                subjects: self.subjects.clone(),
                partition,
                keys: Arc::downgrade(&keys),
                subject: Arc::downgrade(&subject),
            }))
        })
    }

    /// The source provider owns admission, including the native Pool mutex.
    /// Actions must be synchronous: native ownership cannot change until return.
    pub(crate) fn with_current<T>(
        &self,
        action: impl FnOnce() -> Result<T, HubError>,
    ) -> Result<T, HubError> {
        let subjects = self.subjects.upgrade().ok_or(HubError::Stale)?;
        let expected = self.subject()?;
        let mut action = Some(action);
        let mut result = None;
        subjects.with_current(&self.hub, &expected, &mut || {
            let run = action.take().ok_or(HubError::Stale)?;
            result = Some(run()?);
            Ok(())
        })?;
        result.ok_or(HubError::Stale)
    }

    pub(crate) async fn during<T>(
        &self,
        future: impl std::future::Future<Output = T>,
    ) -> Result<T, HubError> {
        crate::checked::during(|| self.check(), future).await
    }
}
