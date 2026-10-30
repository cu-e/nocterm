//! Original normal account metadata is available before any Cloud authority pin.
use super::*;
use crate::{account::operations::AccountOperations, pool::Pool};
use nocterm_hubs_model::source_identity::LocalAccountSource;
use std::sync::Weak;

pub(crate) struct CloudTrustCapture {
    handle: Handle,
    pool: Weak<Pool>,
    operations: Arc<AccountOperations>,
}
impl CloudTrustCapture {
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
        resident: AllocationLease,
    ) -> Result<AccountCloudTrust, CloudTrustError> {
        require(&resident, CLOUD_TRUST_RESIDENT_BYTES)?;
        let result = (|| {
            let bound = self.operations.bind(restriction).map_err(account)?;
            let client = CapturedClient::account(
                self.pool.upgrade().ok_or(CloudTrustError::Stale)?,
                bound.source().native(),
                bound.source().clone(),
            )
            .map_err(account)?;
            let sdk = client.captured_cloud().map_err(account)?;
            let normal = codec::NormalAccount::capture(&bound, &sdk.info().hub_id)?;
            client.check_cloud().map_err(account)?;
            bound
                .source()
                .publish(|| AccountCloudTrust {
                    bound: bound.clone(),
                    client: Arc::new(client),
                    normal,
                    handle: self.handle.clone(),
                    _allocation: resident.clone(),
                })
                .map_err(account)
        })();
        result.map_err(|error| retained(error, resident))
    }
}
