//! The account's captured Cloud access under its pinned hub authority, as runner enrolment
//! and the trusted runner registry use it. Chat material and grants are the contract device
//! core's (`CreateChat`, `OpenChat`, ADR 0136). The handle retains weak native key ownership
//! and cannot revive after lock or replacement sign-in.
mod capture;
mod failure;
pub use failure::AccountMaterialFailure;
mod reservations;
pub use reservations::AccountMaterialReservations;
#[cfg(test)]
mod tests;

use crate::account::operations::BoundOperations;
use nocterm_core::{SharedCause, allocation::AllocationLease};
use nocterm_hub_client::cloud::CloudClient;
use nocterm_hubs_model::HubError;
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
