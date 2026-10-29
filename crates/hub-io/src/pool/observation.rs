//! A reconnect observes one original native owner, including an initially absent one.
use super::{AccountSubject, Pool};
use crate::account::{Slot, SlotReceipt};
use crate::{
    classify::credential_file,
    credentials::{CredentialFile, CredentialLoginId},
};
use nocterm_hubs_model::{HubError, HubId};
use std::sync::{Arc, Mutex, PoisonError, Weak};

#[derive(Clone)]
pub(crate) struct ObservedLogin {
    subject: Option<Weak<AccountSubject>>,
    receipt: SlotReceipt,
}
impl ObservedLogin {
    pub(super) fn new(subject: Option<&Arc<AccountSubject>>, receipt: SlotReceipt) -> Self {
        Self {
            subject: subject.map(Arc::downgrade),
            receipt,
        }
    }
    pub(super) fn subject(&self) -> Result<Option<Arc<AccountSubject>>, HubError> {
        self.subject
            .as_ref()
            .map(|subject| subject.upgrade().ok_or(HubError::Stale))
            .transpose()
    }
    pub(crate) fn receipt(&self) -> &SlotReceipt {
        &self.receipt
    }
}

/// Retirement can close the local owner even when a disk cleanup reports an error.
pub(crate) struct MembershipEnd {
    observed: ObservedLogin,
    result: Result<(), HubError>,
}
impl MembershipEnd {
    pub(super) fn new(observed: ObservedLogin, result: Result<(), HubError>) -> Self {
        Self { observed, result }
    }
    pub(crate) fn into_parts(self) -> (ObservedLogin, Result<(), HubError>) {
        (self.observed, self.result)
    }
}

pub(crate) struct LoginObservation {
    pool: Arc<Pool>,
    hub: HubId,
    slot: Arc<Slot>,
    expected: Mutex<ObservedLogin>,
}
impl LoginObservation {
    pub(crate) fn capture(pool: Arc<Pool>, hub: HubId) -> Result<Arc<Self>, HubError> {
        let (slot, expected) = pool.observe(&hub)?;
        Ok(Arc::new(Self {
            pool,
            hub,
            slot,
            expected: Mutex::new(expected),
        }))
    }
    pub(crate) fn hub(&self) -> &HubId {
        &self.hub
    }
    pub(crate) fn slot(&self) -> &Arc<Slot> {
        &self.slot
    }
    pub(crate) fn snapshot(&self) -> ObservedLogin {
        self.expected
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
    pub(crate) fn published(&self, expected: ObservedLogin) {
        *self.expected.lock().unwrap_or_else(PoisonError::into_inner) = expected;
    }
    pub(crate) fn ended_absence(&self, receipt: SlotReceipt) {
        self.published(ObservedLogin::new(None, receipt));
    }
    pub(crate) fn original_subject(&self) -> Result<Option<Arc<AccountSubject>>, HubError> {
        self.snapshot().subject()
    }
    pub(crate) fn became(&self, subject: &Arc<AccountSubject>) -> Result<(), HubError> {
        self.published(self.pool.observe_original(&self.hub, subject)?);
        Ok(())
    }
    pub(crate) fn retire(
        &self,
        file: &CredentialFile,
        subject: Option<(&str, &str)>,
        login: Option<CredentialLoginId>,
        erase: bool,
    ) -> Result<(), HubError> {
        if let Some(subject) = self.original_subject()? {
            let (next, result) = self
                .pool
                .end_membership(&self.hub, &subject, !erase)?
                .into_parts();
            self.published(next);
            return result;
        }
        let expected = self.snapshot();
        let next = self.with_current(|| {
            let mut slot_error = None;
            let owned = match subject {
                Some((account, device)) => file
                    .retire_if(account, device, login, erase, || {
                        match self.slot.end_observed(expected.receipt(), erase) {
                            Ok(ended) => {
                                let (receipt, result) = ended.into_parts();
                                self.ended_absence(receipt);
                                slot_error = result.err();
                            }
                            Err(error) => slot_error = Some(error),
                        }
                    })
                    .map_err(credential_file)?,
                None => false,
            };
            if !owned {
                let (receipt, _) = self
                    .slot
                    .end_observed(expected.receipt(), false)?
                    .into_parts();
                self.ended_absence(receipt);
                return Err(HubError::Stale);
            }
            if let Some(error) = slot_error {
                return Err(error);
            }
            Ok(ObservedLogin::new(None, self.slot.observed()))
        })?;
        self.published(next);
        Ok(())
    }
    pub(crate) fn check(&self) -> Result<(), HubError> {
        self.pool
            .with_observed(&self.hub, &self.slot, &self.snapshot(), || Ok(()))
    }
    pub(crate) fn with_current<T>(
        &self,
        action: impl FnOnce() -> Result<T, HubError>,
    ) -> Result<T, HubError> {
        self.pool
            .with_observed(&self.hub, &self.slot, &self.snapshot(), action)
    }
    pub(crate) async fn during<T>(
        &self,
        future: impl std::future::Future<Output = T>,
    ) -> Result<T, HubError> {
        let mut future = std::pin::pin!(future);
        std::future::poll_fn(|cx| {
            if let Err(error) = self.check() {
                return std::task::Poll::Ready(Err(error));
            }
            match future.as_mut().poll(cx) {
                std::task::Poll::Pending => std::task::Poll::Pending,
                std::task::Poll::Ready(value) => {
                    std::task::Poll::Ready(self.check().map(|()| value))
                }
            }
        })
        .await
    }
}
