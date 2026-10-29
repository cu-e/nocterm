//! One original SDK client and native membership, shared by captured I/O consumers.
use super::{Pool, subject_of};
use crate::{
    account::{
        OpenKeys, Slot,
        source::{AccountSubject, AccountSubjects, CapturedAccount},
    },
    classify::classify,
};
use nocterm_hub_client::{ClientError, HubClient, StatusCode};
use nocterm_hubs_model::{HubError, HubId, source_identity::LocalAccountSource};
use std::sync::{Arc, Weak};

struct KeyOwner {
    slot: Weak<Slot>,
    keys: Weak<OpenKeys>,
}
pub(crate) struct CapturedClient {
    pool: Arc<Pool>,
    hub: HubId,
    client: HubClient,
    subject: Weak<AccountSubject>,
    source: Option<Arc<dyn LocalAccountSource>>,
    owner: Option<KeyOwner>,
}
impl CapturedClient {
    /// Capture the actual signed-in requester now, before any worker is spawned.
    pub(crate) fn current(pool: Arc<Pool>, hub: HubId) -> Result<Self, HubError> {
        let subject = pool.current(&hub).ok_or(HubError::NotSignedIn)?;
        let client = pool.captured(&hub, &subject)?;
        let owner = pool.accounts.slot(&hub).ok().and_then(|slot| {
            let keys = slot.open()?;
            (keys.keyset().account_id() == subject.account()).then(|| KeyOwner {
                slot: Arc::downgrade(&slot),
                keys: Arc::downgrade(&keys),
            })
        });
        Ok(Self {
            pool,
            hub,
            client,
            subject: Arc::downgrade(&subject),
            source: None,
            owner,
        })
    }
    pub(crate) fn account(
        pool: Arc<Pool>,
        native: &CapturedAccount,
        source: Arc<dyn LocalAccountSource>,
    ) -> Result<Self, HubError> {
        source.check_current().map_err(|_| HubError::NotSignedIn)?;
        native.check_current().map_err(|_| HubError::NotSignedIn)?;
        if source.partition() != native.partition() {
            return Err(HubError::NotSignedIn);
        }
        let slot = native.slot()?;
        let keys = native.keys()?;
        let subject = native.subject()?;
        let hub = slot.hub().clone();
        let client = pool.captured(&hub, &subject)?;
        Ok(Self {
            pool,
            hub,
            client,
            subject: Arc::downgrade(&subject),
            source: Some(source),
            owner: Some(KeyOwner {
                slot: Arc::downgrade(&slot),
                keys: Arc::downgrade(&keys),
            }),
        })
    }
    pub(crate) fn hub(&self) -> &HubId {
        &self.hub
    }
    pub(crate) fn check(&self) -> Result<(), HubError> {
        if self.pool.is_stopped() {
            return Err(HubError::NotSignedIn);
        }
        if let Some(source) = &self.source {
            source.check_current().map_err(|_| HubError::NotSignedIn)?;
        }
        let expected = self.subject.upgrade().ok_or(HubError::NotSignedIn)?;
        self.pool.captured(&self.hub, &expected)?;
        if subject_of(&self.client).as_ref() != Some(expected.as_ref()) {
            return Err(HubError::NotSignedIn);
        }
        if self.source.is_some()
            && let Some(owner) = &self.owner
        {
            let slot = owner.slot.upgrade().ok_or(HubError::NotSignedIn)?;
            let keys = owner.keys.upgrade().ok_or(HubError::KeysLocked)?;
            if !slot
                .open()
                .is_some_and(|current| Arc::ptr_eq(&current, &keys))
            {
                return Err(HubError::NotSignedIn);
            }
        }
        Ok(())
    }
    /// Actual worker admission, including every resume after an awaited operation.
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
    /// No implicit network refresh. Ordinary native authentication owns refresh
    /// of these shared credentials; this access cannot retarget another login.
    pub(crate) fn captured_cloud(&self) -> Result<HubClient, HubError> {
        self.check_cloud()?;
        Ok(self.client.clone())
    }
    pub(crate) fn check_cloud(&self) -> Result<(), HubError> {
        self.client
            .require_cloud_metadata()
            .map_err(|error| self.failure(error))?;
        self.check()
    }
    pub(crate) async fn during_cloud<T>(
        &self,
        future: impl std::future::Future<Output = T>,
    ) -> Result<T, HubError> {
        let mut future = std::pin::pin!(future);
        std::future::poll_fn(|cx| {
            if let Err(error) = self.check_cloud() {
                return std::task::Poll::Ready(Err(error));
            }
            match future.as_mut().poll(cx) {
                std::task::Poll::Pending => std::task::Poll::Pending,
                std::task::Poll::Ready(value) => {
                    std::task::Poll::Ready(self.check_cloud().map(|()| value))
                }
            }
        })
        .await
    }
    pub(crate) async fn fresh(&self) -> Result<HubClient, HubError> {
        self.check()?;
        self.during(self.client.ensure_fresh())
            .await?
            .map_err(|error| self.failure(error))?;
        self.check()?;
        Ok(self.client.clone())
    }
    pub(crate) fn failure(&self, error: ClientError) -> HubError {
        if matches!(error, ClientError::Revoked)
            && let Some(subject) = self.subject.upgrade()
        {
            let original = self
                .owner
                .as_ref()
                .and_then(|owner| Some((owner.slot.upgrade()?, owner.keys.upgrade()?)));
            if let Some((slot, keys)) = original {
                self.pool.retire_captured(&self.hub, &subject, &slot, &keys);
            } else {
                self.pool.retire_membership(&self.hub, &subject);
            }
        }
        if matches!(
            &error,
            ClientError::NotSignedIn
                | ClientError::Status {
                    code: StatusCode::Unauthenticated,
                    ..
                }
        ) {
            let Some(subject) = self.subject.upgrade() else {
                return HubError::Stale;
            };
            // Authentication rejection ends only the original local login. Unlike
            // an explicit Revoked code, expiry keeps the device key and ciphertext.
            let ended = match self.pool.end_membership(&self.hub, &subject, true) {
                Ok(ended) => ended,
                Err(error) => return error,
            };
            let (_, retired) = ended.into_parts();
            if let Err(error) = retired {
                return error;
            }
        }
        classify(self.hub.address(), error)
    }
}
