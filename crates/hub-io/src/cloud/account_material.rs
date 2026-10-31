//! Account-owned Cloud chat material restored by ordinary account keyset unlock.
//! Only ciphertext survives an operation; an opaque material handle retains weak
//! native key ownership and cannot revive after lock or replacement sign-in.
mod capture;
mod failure;
mod grant;
pub use failure::AccountMaterialFailure;
pub use grant::StoredChatKeyGrant;
mod operation;
mod reservations;
pub use reservations::AccountMaterialReservations;
mod record;
mod storage;
#[cfg(test)]
mod tests;

use crate::account::operations::BoundOperations;
use futures::future::BoxFuture;
use nocterm_cloud_model::{authority::StreamBinding, identity::ChatKeyEpoch};
use nocterm_core::{
    SharedCause,
    allocation::{Allocated, AllocationLease},
};
use nocterm_hub_client::cloud::CloudClient;
use nocterm_hubs_model::HubError;
use nocterm_keys::cloud::ChatKey;
use std::{error::Error, sync::Arc};
use tokio::runtime::Handle;

pub(crate) use capture::CloudMaterialCapture;
/// Independently retained runner pins, never adopted from an unsolicited reply.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChatMaterialPins {
    pub encryption: [u8; 32],
    pub signing: [u8; 32],
    pub proof: [u8; 32],
}
/// Descriptive selectors; only the captured native/SDK owner authenticates them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatMaterialRequest {
    pub stream: StreamBinding,
    pub epoch: ChatKeyEpoch,
    pub pins: ChatMaterialPins,
}
#[derive(Debug, thiserror::Error)]
pub enum AccountMaterialError {
    #[error("Cloud chat material belongs to another source, epoch or pinned runner")]
    Binding,
    #[error("Cloud chat material exceeds its finite allocation or format bound")]
    Capacity,
    #[error("Cloud chat material is missing or was deleted")]
    Missing,
    #[error("Cloud chat material is malformed")]
    Format,
    #[error("creating or rotating this material requires an explicit supported transition")]
    Rotation,
    #[error("the captured native account is no longer available")]
    Stale,
    #[error("the hub does not support Cloud chat material")]
    Unsupported,
    #[error(transparent)]
    Account(#[from] HubError),
    #[error("Cloud material authentication or transport failed")]
    Backend(#[source] SharedCause),
    /// Foreign transport errors can retain decoded metadata. Keep their original
    /// work permit until that actual cause is displayed/discarded by its caller.
    #[error(transparent)]
    Retained(AccountMaterialFailure),
}
// A 16KiB protobuf response can contain thousands of empty Collection entries.
// Account for geometric Vec capacity before its decoder allocates, plus bounded
// string/crypto/RPC generations. Tonic/TLS/runtime baseline is not part of this
// logical response/work allowance. No idle material reserves a worker allowance.
pub(super) const MAX_RESPONSE: usize = 16 * 1024;
pub const ACCOUNT_MATERIAL_RESIDENT_BYTES: usize = 16 * 1024;
pub const ACCOUNT_MATERIAL_WORK_BYTES: usize = (MAX_RESPONSE + 2)
    * std::mem::size_of::<nocterm_hub_proto::v1::Collection>()
    + 32 * MAX_RESPONSE
    + 64 * 1024;

/// Store wire/proof plus decoded native container/crypto peak. The SDK owns its
/// transport bound; this adapter separately owns protobuf DTO expansion.
pub fn account_material_work_bytes() -> Result<usize, AccountMaterialError> {
    ACCOUNT_MATERIAL_WORK_BYTES
        .checked_add(nocterm_hub_client::store_work_bytes(MAX_RESPONSE).map_err(cause)?)
        .ok_or(AccountMaterialError::Capacity)
}

pub struct CloudAccountMaterial {
    handle: Handle,
    bound: BoundOperations,
    cloud: Arc<CloudClient>,
    authority: [u8; 32],
    _allocation: AllocationLease,
}
impl CloudAccountMaterial {
    /// Read-only original owners for the native conversation adapter. No keys,
    /// selectors, new capture or replacement-login lookup crosses this seam.
    pub(in crate::cloud) fn conversation_parts(
        &self,
    ) -> (BoundOperations, Arc<CloudClient>, [u8; 32]) {
        (self.bound.clone(), self.cloud.clone(), self.authority)
    }

    pub fn create(
        &self,
        request: ChatMaterialRequest,
        reservations: AccountMaterialReservations,
    ) -> BoxFuture<'static, Result<AccountChatMaterial, AccountMaterialError>> {
        self.run(request, reservations, true)
    }
    pub fn open(
        &self,
        request: ChatMaterialRequest,
        reservations: AccountMaterialReservations,
    ) -> BoxFuture<'static, Result<AccountChatMaterial, AccountMaterialError>> {
        self.run(request, reservations, false)
    }
    fn run(
        &self,
        request: ChatMaterialRequest,
        reservations: AccountMaterialReservations,
        create: bool,
    ) -> BoxFuture<'static, Result<AccountChatMaterial, AccountMaterialError>> {
        let prepared: Result<_, AccountMaterialError> = (|| {
            reservations.require(account_material_work_bytes()?)?;
            self.cloud.check_source().map_err(cause)?;
            capture::require_request(&self.bound, &self.cloud, self.authority, &request)?;
            let work = self
                .bound
                .capture_bounded(MAX_RESPONSE, reservations.work())?;
            Ok((work, self.bound.clone(), self.cloud.clone()))
        })();
        let (work, bound, cloud) = match prepared {
            Ok(value) => value,
            Err(error) => {
                let error = retain_error(error, reservations.work());
                return Box::pin(async move { Err(error) });
            }
        };
        let source = Allocated::new(bound.source().clone(), reservations.resident());
        let operation = Allocated::new(
            operation::MaterialOperation::new(work, request, bound, cloud, reservations.resident()),
            reservations.work(),
        );
        let worker = crate::task::spawn(&self.handle, async move {
            operation
                .value()
                .execute(create)
                .await
                .map_err(|error| retain_error(error, operation.allocation().clone()))
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
/// No plaintext key or strong OpenKeys survives in this handle. Its reservation
/// remains alive until the actual wrapped bytes and selectors have been dropped.
pub struct AccountChatMaterial {
    resident: Allocated<MaterialOwner>,
}
struct MaterialOwner {
    record: record::MaterialRecord,
    bound: BoundOperations,
    cloud: Arc<CloudClient>,
}
impl AccountChatMaterial {
    pub fn request(&self) -> &ChatMaterialRequest {
        self.resident.value().record.request()
    }
    pub(super) fn check_current(&self) -> Result<(), AccountMaterialError> {
        let owner = self.resident.value();
        owner.cloud.check_source().map_err(cause)?;
        owner
            .bound
            .source()
            .check()
            .map_err(AccountMaterialError::from)
    }
    pub(super) fn with_key<T>(
        &self,
        use_key: impl FnOnce(&ChatKey) -> Result<T, AccountMaterialError>,
    ) -> Result<T, AccountMaterialError> {
        let owner = self.resident.value();
        self.check_current()?;
        storage::with_key(owner.bound.source(), &owner.record, use_key)
    }
}

pub(super) fn retain_error(
    error: AccountMaterialError,
    allocation: AllocationLease,
) -> AccountMaterialError {
    AccountMaterialError::Retained(AccountMaterialFailure::new(error, allocation))
}

fn cause(error: impl Error + Send + Sync + 'static) -> AccountMaterialError {
    AccountMaterialError::Backend(SharedCause::new(error))
}
