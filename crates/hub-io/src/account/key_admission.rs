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
    pub(crate) async fn confirm(&self) -> Result<(), HubError> {
        let slot = self.member.slot()?;
        let _turn = slot.cache_turn().await;
        loop {
            self.check()?;
            let original = slot.cache_snapshot();
            let mut next = original.as_deref().cloned().ok_or(HubError::NotFound)?;
            next.recovery_unconfirmed = false;
            let draft = slot.stage_cache(original, next).await?;
            let committed = self.member.with_current(|| {
                slot.commit_member_cache(&self.member.subject()?, &self.receipt, draft, true)
            })?;
            if let Some((receipt, committed)) = committed {
                slot.finish_cache(committed).await;
                self.member
                    .with_current(|| slot.check_receipt(&self.member.subject()?, &receipt))?;
                slot.notice().raise();
                return Ok(());
            }
        }
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
    pub(crate) fn recovery_unconfirmed(&self) -> Result<bool, HubError> {
        let receipt = self.receipt();
        self.member.with_current(|| {
            self.member
                .slot()?
                .pending_recovery_for(&self.member.subject()?, &receipt)
        })
    }

    pub(crate) fn remember_pending_recovery(&self) -> Result<(), HubError> {
        let receipt = self.receipt();
        let next = self.member.with_current(|| {
            self.member
                .slot()?
                .remember_pending_recovery(&self.member.subject()?, &receipt)
        })?;
        self.replaced(next);
        Ok(())
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
    pub(crate) async fn install(
        &self,
        keys: OpenKeys,
        mut cache: AccountCache,
    ) -> Result<(), HubError> {
        let slot = self.member.slot()?;
        let _turn = slot.cache_turn().await;
        let receipt = self.receipt();
        self.check()?;
        let original = slot.cache_snapshot();
        if let Some(earlier) = original
            .as_deref()
            .filter(|old| old.account_id == cache.account_id)
        {
            cache.inherit_pins(earlier);
            cache.vault = earlier.vault.clone();
        }
        let keys = slot.noticed_keys(keys, &cache);
        let draft = slot.stage_cache(original, cache).await?;
        let committed = self
            .member
            .with_current(|| slot.commit_install(&self.member.subject()?, &receipt, keys, draft))?
            .ok_or(HubError::Stale)?;
        self.replaced(committed.0);
        slot.finish_cache(committed.1).await;
        slot.notice().raise();
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
    pub(crate) async fn update_cache(
        &self,
        change: impl Fn(&mut AccountCache),
    ) -> Result<(), HubError> {
        self.change_cache(change, false).await
    }

    async fn change_cache(
        &self,
        change: impl Fn(&mut AccountCache),
        rotate: bool,
    ) -> Result<(), HubError> {
        let slot = self.member.slot()?;
        let _turn = slot.cache_turn().await;
        let receipt = self.receipt();
        loop {
            self.check()?;
            let original = slot.cache_snapshot();
            let mut next = original.as_deref().cloned().ok_or(HubError::NotFound)?;
            change(&mut next);
            let draft = slot.stage_cache(original, next).await?;
            let committed = self.member.with_current(|| {
                slot.commit_member_cache(&self.member.subject()?, &receipt, draft, rotate)
            })?;
            if let Some((receipt, committed)) = committed {
                self.replaced(receipt);
                slot.finish_cache(committed).await;
                if rotate {
                    slot.notice().raise();
                }
                return Ok(());
            }
        }
    }
    pub(crate) async fn begin_recovery_issue(&self) -> Result<(), HubError> {
        self.change_cache(|cache| cache.recovery_unconfirmed = true, true)
            .await
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
