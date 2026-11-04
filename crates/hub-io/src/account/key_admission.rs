//! One bounded key operation; display capabilities keep immutable snapshots, not this owner.
use super::{OpenKeys, Recovery, SlotReceipt, source::CapturedMembership};
use crate::{cache::AccountCache, pool::captured::CapturedClient};
use nocterm_hub_client::{ClientError, HubClient};
use nocterm_hubs_model::HubError;
use std::sync::{Arc, Mutex, PoisonError};

#[derive(Clone)]
enum Requester {
    Native(Arc<CapturedClient>),
    #[cfg(test)]
    Scripted,
}

/// Unlike a running operation, an offer's immutable snapshot never follows publication.
#[derive(Clone)]
pub(crate) struct KeySnapshot {
    member: Arc<CapturedMembership>,
    receipt: SlotReceipt,
}
impl KeySnapshot {
    pub(crate) fn check(&self) -> Result<(), HubError> {
        self.member.with_current(|| {
            self.member
                .slot()?
                .check_receipt(&self.member.subject()?, &self.receipt)
        })
    }
    pub(crate) fn member(&self) -> &Arc<CapturedMembership> {
        &self.member
    }
    pub(crate) fn confirm(&self) -> Result<(), HubError> {
        self.member.with_current(|| {
            self.member
                .slot()?
                .ack_recovery(&self.member.subject()?, &self.receipt)
        })
    }
}

pub(crate) struct KeyAdmission {
    member: Arc<CapturedMembership>,
    receipt: Mutex<SlotReceipt>,
    requester: Requester,
}
impl KeyAdmission {
    pub(crate) fn native(
        member: Arc<CapturedMembership>,
        client: Arc<CapturedClient>,
    ) -> Result<Arc<Self>, HubError> {
        let receipt = member.with_current(|| member.slot()?.receipt(&member.subject()?))?;
        let admission = Arc::new(Self {
            member,
            receipt: Mutex::new(receipt),
            requester: Requester::Native(client),
        });
        admission.check()?;
        Ok(admission)
    }
    pub(crate) fn bound(
        snapshot: KeySnapshot,
        client: Arc<CapturedClient>,
    ) -> Result<Arc<Self>, HubError> {
        snapshot.check()?;
        let admission = Arc::new(Self {
            member: snapshot.member,
            receipt: Mutex::new(snapshot.receipt),
            requester: Requester::Native(client),
        });
        admission.check()?;
        Ok(admission)
    }
    #[cfg(test)]
    pub(crate) fn scripted(member: Arc<CapturedMembership>) -> Arc<Self> {
        let receipt = member
            .with_current(|| member.slot()?.receipt(&member.subject()?))
            .unwrap();
        Arc::new(Self {
            member,
            receipt: Mutex::new(receipt),
            requester: Requester::Scripted,
        })
    }
    fn receipt(&self) -> SlotReceipt {
        self.receipt
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
    fn replaced(&self, receipt: SlotReceipt) {
        *self.receipt.lock().unwrap_or_else(PoisonError::into_inner) = receipt;
    }
    pub(crate) fn snapshot(&self) -> Result<KeySnapshot, HubError> {
        let snapshot = KeySnapshot {
            member: self.member.clone(),
            receipt: self.receipt(),
        };
        snapshot.check()?;
        Ok(snapshot)
    }
    pub(crate) fn member(&self) -> &Arc<CapturedMembership> {
        &self.member
    }
    pub(crate) fn check(&self) -> Result<(), HubError> {
        match &self.requester {
            Requester::Native(client) => client.check()?,
            #[cfg(test)]
            Requester::Scripted => {}
        }
        self.snapshot().map(|_| ())
    }
    pub(crate) fn cache(&self) -> Result<AccountCache, HubError> {
        let receipt = self.receipt();
        self.member.with_current(|| {
            self.member
                .slot()?
                .cache_for(&self.member.subject()?, &receipt)
        })
    }
    pub(crate) fn keys(&self) -> Result<Arc<OpenKeys>, HubError> {
        self.check()?;
        let receipt = self.receipt();
        self.member.with_current(|| {
            let slot = self.member.slot()?;
            let subject = self.member.subject()?;
            slot.check_receipt(&subject, &receipt)?;
            slot.open_for(&subject)
        })
    }
    pub(crate) fn install(&self, keys: OpenKeys, cache: AccountCache) -> Result<(), HubError> {
        let receipt = self.receipt();
        let next = self.member.with_current(|| {
            self.member
                .slot()?
                .install_if(&self.member.subject()?, &receipt, keys, cache)
        })?;
        self.replaced(next);
        Ok(())
    }
    pub(crate) fn recovering(&self, recovery: Recovery) -> Result<(), HubError> {
        let receipt = self.receipt();
        let next = self.member.with_current(|| {
            self.member
                .slot()?
                .recovering_if(&self.member.subject()?, &receipt, recovery)
        })?;
        self.replaced(next);
        Ok(())
    }
    pub(crate) fn recovery(&self) -> Result<Arc<Recovery>, HubError> {
        let receipt = self.receipt();
        self.member.with_current(|| {
            let slot = self.member.slot()?;
            slot.check_receipt(&self.member.subject()?, &receipt)?;
            slot.recovery().ok_or(HubError::NoRecoveryPending)
        })
    }
    pub(crate) fn update_cache(
        &self,
        change: impl FnOnce(&mut AccountCache),
    ) -> Result<(), HubError> {
        let receipt = self.receipt();
        self.member.with_current(|| {
            self.member
                .slot()?
                .change_cache_if(&self.member.subject()?, &receipt, change)
        })
    }
    pub(crate) fn begin_recovery_issue(&self) -> Result<(), HubError> {
        let receipt = self.receipt();
        let next = self.member.with_current(|| {
            self.member
                .slot()?
                .begin_recovery_issue(&self.member.subject()?, &receipt)
        })?;
        self.replaced(next);
        Ok(())
    }
    pub(crate) async fn fresh(&self) -> Result<HubClient, HubError> {
        self.check()?;
        let client = match &self.requester {
            Requester::Native(client) => client.fresh().await?,
            #[cfg(test)]
            Requester::Scripted => return Err(HubError::NotSignedIn),
        };
        self.check()?;
        Ok(client)
    }
    pub(crate) fn failure(&self, error: ClientError) -> HubError {
        match &self.requester {
            Requester::Native(client) => client.failure(error),
            #[cfg(test)]
            Requester::Scripted => crate::classify::classify(self.member.hub().address(), error),
        }
    }

    pub(crate) async fn during<T>(
        &self,
        future: impl std::future::Future<Output = T>,
    ) -> Result<T, HubError> {
        crate::checked::during(|| self.check(), future).await
    }
}
