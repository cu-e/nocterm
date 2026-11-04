//! Actual native membership and independently pinned SDK capability admission.
use super::*;
use crate::{
    account::operations::AccountOperations,
    pool::{Pool, captured::CapturedClient},
};
use futures::future::BoxFuture;
use nocterm_core::allocation::Allocated;
use nocterm_hub_model::cloud::capabilities::CLOUD_FEATURE_CHAT_KEY_MATERIAL;
use nocterm_hubs_model::source_identity::LocalAccountSource;
use std::sync::Weak;

pub(crate) struct CloudMaterialCapture {
    handle: Handle,
    pool: Weak<Pool>,
    operations: Arc<AccountOperations>,
}
impl CloudMaterialCapture {
    pub(crate) fn new(
        handle: Handle,
        pool: &Arc<Pool>,
        operations: Arc<AccountOperations>,
    ) -> Self {
        Self {
            handle,
            pool: Arc::downgrade(pool),
            operations,
        }
    }
    pub(crate) fn capture(
        &self,
        restriction: Arc<dyn LocalAccountSource>,
        authority: [u8; 32],
        reservations: AccountMaterialReservations,
    ) -> BoxFuture<'static, Result<CloudAccountMaterial, AccountMaterialError>> {
        let prepared: Result<_, AccountMaterialError> = (|| {
            reservations.require(nocterm_hub_client::cloud::capability_work_bytes())?;
            if reservations.resident().bytes()
                < nocterm_hub_client::cloud::capability_resident_bytes()
            {
                return Err(AccountMaterialError::Capacity);
            }
            let bound = self.operations.bind(restriction)?;
            let client = CapturedClient::account(
                self.pool.upgrade().ok_or(AccountMaterialError::Stale)?,
                bound.source().native(),
                bound.source().clone(),
            )?;
            client.check_cloud()?;
            Ok((bound, client))
        })();
        let (bound, client) = match prepared {
            Ok(value) => value,
            Err(error) => {
                let error = retain_error(error, reservations.work());
                return Box::pin(async move { Err(error) });
            }
        };
        let source = Allocated::new(bound.source().clone(), reservations.resident());
        let operation = CaptureOperation {
            client,
            bound,
            authority,
            resident: reservations.resident(),
            work: reservations.work(),
        };
        let worker = crate::task::spawn(&self.handle, async move {
            operation
                .execute()
                .await
                .map_err(|error| retain_error(error, operation.work.clone()))
        });
        Box::pin(async move {
            let result = source
                .value()
                .during(worker)
                .await?
                .ok_or(AccountMaterialError::Stale)??;
            source
                .value()
                .publish(|| result)
                .map_err(AccountMaterialError::from)
        })
    }
}
// All captured inputs precede the work guard. The async method borrows this
// intact owner through every fallible await; the SDK also owns its same lease.
struct CaptureOperation {
    client: CapturedClient,
    bound: BoundOperations,
    authority: [u8; 32],
    resident: AllocationLease,
    work: AllocationLease,
}
impl CaptureOperation {
    async fn execute(&self) -> Result<CloudAccountMaterial, AccountMaterialError> {
        let sdk = self.client.captured_cloud()?;
        let offered = self
            .client
            .during_cloud(sdk.cloud_offer_with_residency(Arc::new(self.work.clone())))
            .await?
            .map_err(cause)?;
        let cloud = offered
            .accept_with_residency(&self.authority, Arc::new(self.resident.clone()))
            .map_err(cause)?;
        self.client.check_cloud()?;
        if cloud.limits().features & CLOUD_FEATURE_CHAT_KEY_MATERIAL == 0 {
            return Err(AccountMaterialError::Unsupported);
        }
        let subject = self.bound.source().native().subject()?;
        if subject.account() != cloud.subject().account().as_str()
            || subject.device() != cloud.subject().device().as_str()
        {
            return Err(AccountMaterialError::Binding);
        }
        self.bound.source().check()?;
        Ok(CloudAccountMaterial {
            bound: self.bound.clone(),
            cloud: Arc::new(cloud),
            authority: self.authority,
            _allocation: self.resident.clone(),
        })
    }
}
