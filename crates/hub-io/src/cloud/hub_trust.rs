//! Account-encrypted immutable hub pin, located before Cloud capability admission.
mod capture;
mod operation;
mod records;
mod setup;
mod storage;
#[cfg(test)]
mod tests;
use super::{AccountMaterialError, RunnerRegistration};
use crate::{account::operations::BoundOperations, pool::captured::CapturedClient};
pub(crate) use capture::CloudTrustCapture;
use futures::future::BoxFuture;
use nocterm_cloud_model::identity::AccountScope;
use nocterm_core::{
    SharedCause,
    allocation::{Allocated, AllocationLease},
};
use nocterm_hubs_model::HubError;
use sha2::Digest as _;
use std::{error::Error, sync::Arc};
use tokio::runtime::Handle;

pub const CLOUD_TRUST_RESIDENT_BYTES: usize = 16 * 1024;
const MAX_RESPONSE: usize = 16 * 1024;
/// Counts guarded Store work and native collection/decode/crypto generations;
/// connection/TLS/runtime baseline and caller-owned setup input are separate.
pub fn cloud_trust_work_bytes() -> Result<usize, CloudTrustError> {
    let collections = (MAX_RESPONSE + 2) * std::mem::size_of::<nocterm_hub_proto::v1::Collection>();
    Ok(collections
        + 2 * CLOUD_TRUST_RESIDENT_BYTES
        + 2 * records::MAX_PLAINTEXT
        + 2 * (records::MAX_PLAINTEXT + 42)
        + 2 * MAX_RESPONSE
        + nocterm_hub_client::cloud::capability_work_bytes()
        + nocterm_hub_client::store_work_bytes(MAX_RESPONSE).map_err(backend)?)
}
#[derive(Debug, thiserror::Error)]
pub enum CloudTrustError {
    #[error("Cloud has not been configured for this account and hub")]
    NotConfigured,
    #[error("the saved Cloud hub trust was deleted")]
    Deleted,
    #[error("the saved Cloud hub trust does not match the original account or origin")]
    Binding,
    #[error("the Cloud hub trust record is malformed")]
    Format,
    #[error("the Cloud hub trust exceeds its allocation bound")]
    Capacity,
    #[error("a different hub authority requires an explicit supported rotation")]
    RotationRequired,
    #[error("the original Cloud account trust source is unavailable")]
    Stale,
    #[error("this hub does not support the required Cloud material capability")]
    Unsupported,
    #[error("the Cloud account trust operation failed")]
    Storage(#[source] AccountMaterialError),
}
/// Imported public setup values. Construction checks shape, not the independent
/// comparison with the trusted runner operator required before registration.
pub struct CloudRunnerSetup {
    origin: nocterm_hubs_model::HubId,
    raw_hub: String,
    authority: [u8; 32],
    runner: RunnerRegistration,
}
impl CloudRunnerSetup {
    /// Decode the bounded public CLI setup document. The caller owns the input
    /// and setup allocation; original-account validation still happens at import.
    pub fn from_public_toml(bytes: &[u8], label: String) -> Result<Self, CloudTrustError> {
        setup::decode(bytes, label)
    }
    pub fn new(
        origin: nocterm_hubs_model::HubId,
        raw_hub: String,
        authority: [u8; 32],
        runner: RunnerRegistration,
    ) -> Result<Self, CloudTrustError> {
        if raw_hub.is_empty() || raw_hub.len() > 128 {
            return Err(CloudTrustError::Capacity);
        }
        let digest: [u8; 32] = sha2::Sha256::digest(authority).into();
        let instance =
            nocterm_hub_model::cloud::hub_namespace_id(&raw_hub, digest).map_err(backend)?;
        if runner.binding().scope().hub().authority().as_bytes() != &digest
            || runner.binding().scope().hub().instance().as_str() != instance.as_str()
        {
            return Err(CloudTrustError::Binding);
        }
        Ok(Self {
            origin,
            raw_hub,
            authority,
            runner,
        })
    }
    pub fn runner(&self) -> &RunnerRegistration {
        &self.runner
    }
}
/// Actual finite permits supplied before metadata, SDK or crypto copies.
pub struct CloudTrustReservations {
    work: AllocationLease,
    resident: AllocationLease,
}
impl CloudTrustReservations {
    pub fn new(work: AllocationLease, resident: AllocationLease) -> Result<Self, CloudTrustError> {
        require(&resident, CLOUD_TRUST_RESIDENT_BYTES)?;
        require(
            &resident,
            nocterm_hub_client::cloud::capability_resident_bytes(),
        )?;
        require(&work, cloud_trust_work_bytes()?)?;
        Ok(Self { work, resident })
    }
}
/// Weak original-account authority; no account/collection private key is idle here.
pub struct AccountCloudTrust {
    bound: BoundOperations,
    client: Arc<CapturedClient>,
    normal: records::NormalAccount,
    handle: Handle,
    _allocation: AllocationLease,
}
impl AccountCloudTrust {
    pub fn load(
        &self,
        reservations: CloudTrustReservations,
    ) -> BoxFuture<'static, Result<TrustedCloudHub, CloudTrustError>> {
        self.run(None, reservations)
    }
    /// Explicit setup only. A load or ordinary sign-in never creates trust.
    pub fn register_new(
        &self,
        setup: CloudRunnerSetup,
        reservations: CloudTrustReservations,
    ) -> BoxFuture<'static, Result<TrustedCloudHub, CloudTrustError>> {
        self.run(Some(setup), reservations)
    }
    fn run(
        &self,
        setup: Option<CloudRunnerSetup>,
        reservations: CloudTrustReservations,
    ) -> BoxFuture<'static, Result<TrustedCloudHub, CloudTrustError>> {
        let work = reservations.work.clone();
        let prepared = (|| {
            self.client.check_cloud().map_err(account)?;
            self.bound.source().check().map_err(account)?;
            let opened = self
                .bound
                .capture_bounded(MAX_RESPONSE, work.clone())
                .map_err(account)?;
            Ok(operation::TrustOperation::new(
                opened,
                self.bound.clone(),
                self.client.clone(),
                self.normal.clone(),
                setup,
                reservations.resident,
                work.clone(),
            ))
        })();
        let operation = match prepared {
            Ok(operation) => Allocated::new(operation, work.clone()),
            Err(error) => return Box::pin(async move { Err(retained(error, work)) }),
        };
        let source = Allocated::new(self.bound.source().clone(), work.clone());
        let worker = crate::task::spawn(&self.handle, async move {
            operation
                .value()
                .execute()
                .await
                .map_err(|error| retained(error, operation.allocation().clone()))
        });
        Box::pin(async move {
            let result = source
                .value()
                .during(worker)
                .await
                .map_err(account)
                .map_err(|error| retained(error, work.clone()))?
                .ok_or(CloudTrustError::Stale)??;
            source
                .value()
                .publish(|| result)
                .map_err(account)
                .map_err(|error| retained(error, work))
        })
    }
}
struct HubPinData {
    authority: [u8; 32],
    scope: AccountScope,
    bound: BoundOperations,
    client: Arc<CapturedClient>,
}
/// Public metadata authenticated by the original account's private object.
/// Live Cloud capability and fresh R head proofs remain separate requirements.
pub struct TrustedCloudHub {
    data: Allocated<HubPinData>,
}
impl TrustedCloudHub {
    pub fn authority(&self) -> [u8; 32] {
        self.data.value().authority
    }
    pub fn scope(&self) -> &AccountScope {
        &self.data.value().scope
    }
    pub fn check_current(&self) -> Result<(), CloudTrustError> {
        let result = self
            .data
            .value()
            .client
            .check_cloud()
            .map_err(account)
            .and_then(|()| self.data.value().bound.source().check().map_err(account));
        result.map_err(|error| retained(error, self.data.allocation().clone()))
    }
}
fn require(lease: &AllocationLease, bytes: usize) -> Result<(), CloudTrustError> {
    if lease.bytes() < bytes || lease.payloads() == 0 {
        Err(CloudTrustError::Capacity)
    } else {
        Ok(())
    }
}
fn account(error: HubError) -> CloudTrustError {
    CloudTrustError::Storage(AccountMaterialError::Account(error))
}
fn backend(error: impl Error + Send + Sync + 'static) -> CloudTrustError {
    CloudTrustError::Storage(AccountMaterialError::Backend(SharedCause::new(error)))
}
fn retained(error: CloudTrustError, allocation: AllocationLease) -> CloudTrustError {
    match error {
        CloudTrustError::Storage(error) => {
            CloudTrustError::Storage(super::account_material::retain_error(error, allocation))
        }
        error => error,
    }
}
