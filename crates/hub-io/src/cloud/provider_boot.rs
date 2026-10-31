//! Account-captured verification of an installed R's ephemeral provider boot.
#[cfg(test)]
mod tests;
mod verify;
use super::{AccountMaterialError, CloudAccountMaterial, RunnerRegistration};
use futures::future::BoxFuture;
use nocterm_cloud_model::{identity::Timestamp, journal::JournalCatalogBinding, tool::ToolTimer};
use nocterm_core::{
    SharedCause,
    allocation::{Allocated, AllocationLease},
};
use nocterm_hub_client::cloud::CloudClient;
use nocterm_hub_model::cloud::provider::VerifiedBootDescriptor;
use nocterm_hubs_model::HubError;
use std::{error::Error, sync::Arc};
use tokio::runtime::Handle;
pub(super) use verify::BootClock;

use verify::BootVerifier;

/// Captured binding/source plus bounded descriptor data. This is separate from
/// per-request SDK/status work; idle owners reserve no RPC-sized allowance.
pub const PROVIDER_BOOT_RESIDENT_BYTES: usize = 4096;
/// SDK proof/body/status generations, ≤580B signed input and ≤512B canonical
/// verify scratch, plus decoded and copied descriptor namespace/struct metadata.
/// No Tonic/H2/TLS/runtime or whole SSH/process RSS accounting is claimed.
pub fn provider_boot_work_bytes() -> usize {
    nocterm_hub_client::cloud::status_work_bytes()
        + 580
        + 512
        + 3 * (std::mem::size_of::<VerifiedBootDescriptor>() + 2 * 128)
        + std::mem::size_of::<FetchBoot>()
        + std::mem::size_of::<BootFacts>()
}
#[derive(Debug, thiserror::Error)]
pub enum ProviderBootError {
    #[error("the provider boot differs from the installed account/runner pins")]
    Binding,
    #[error("the provider boot exceeds its finite allocation or format bound")]
    Capacity,
    #[error("the signed provider boot window has expired")]
    Expired,
    #[error("the original provider boot clock was fenced")]
    Clock,
    #[error("the original provider boot source is unavailable")]
    Stale,
    #[error("the provider boot codec or transport failed")]
    Storage(#[source] AccountMaterialError),
}
pub struct ProviderBootReservations {
    work: AllocationLease,
    resident: AllocationLease,
}
impl ProviderBootReservations {
    pub fn new(
        work: AllocationLease,
        resident: AllocationLease,
    ) -> Result<Self, ProviderBootError> {
        require_work(&work)?;
        require_resident(&resident)?;
        Ok(Self { work, resident })
    }
}
/// Independently installed RunnerRegistration is supplied by account setup or
/// the authenticated personal roster, never adopted from a status/directory reply.
pub struct AccountProviderBoot {
    verifier: Arc<BootVerifier>,
    cloud: Arc<CloudClient>,
    handle: Handle,
}
impl AccountProviderBoot {
    pub fn capture(
        material: &CloudAccountMaterial,
        registration: &RunnerRegistration,
        allocation: AllocationLease,
        timer: Arc<dyn ToolTimer>,
        handle: Handle,
    ) -> Result<Arc<Self>, ProviderBootError> {
        require_resident(&allocation)?;
        let result = (|| {
            let (bound, cloud, authority) = material.conversation_parts();
            super::runner_registry::validate_scope(
                &bound,
                &cloud,
                authority,
                registration.binding().scope(),
            )
            .map_err(|error| match error {
                super::RunnerRegistryError::Storage(cause) => ProviderBootError::Storage(cause),
                _ => ProviderBootError::Binding,
            })?;
            let verifier = BootVerifier::new(
                bound,
                registration.binding().clone(),
                registration.pins(),
                timer,
                allocation.clone(),
            )?;
            Ok(Arc::new(Self {
                verifier: Arc::new(verifier),
                cloud,
                handle,
            }))
        })();
        result.map_err(|error| retained(error, allocation))
    }
    pub fn fetch(
        self: &Arc<Self>,
        reservations: ProviderBootReservations,
    ) -> BoxFuture<'static, Result<TrustedProviderBoot, ProviderBootError>> {
        let owner = self.clone();
        let outer_work = reservations.work.clone();
        let operation = FetchBoot {
            owner: owner.clone(),
            resident: reservations.resident,
            work: reservations.work,
        };
        let worker = crate::task::spawn(&self.handle, async move {
            operation
                .execute()
                .await
                .map_err(|error| retained(error, operation.work.clone()))
        });
        Box::pin(async move {
            let result = async {
                let result = owner
                    .verifier
                    .source()
                    .during(worker)
                    .await
                    .map_err(account)?
                    .ok_or(ProviderBootError::Stale)??;
                owner.check_current()?;
                let result = owner
                    .verifier
                    .source()
                    .publish(|| result)
                    .map_err(account)?;
                result.check_current()?;
                Ok(result)
            }
            .await;
            result.map_err(|error| retained(error, outer_work))
        })
    }
    fn check_current(&self) -> Result<Timestamp, ProviderBootError> {
        self.cloud.check_source().map_err(backend)?;
        self.verifier.check_current()
    }
}
struct FetchBoot {
    owner: Arc<AccountProviderBoot>,
    resident: AllocationLease,
    work: AllocationLease,
}
impl FetchBoot {
    async fn execute(&self) -> Result<TrustedProviderBoot, ProviderBootError> {
        self.owner.check_current()?;
        let response = self
            .owner
            .cloud
            .runner_status_with_residency(
                *self.owner.verifier.binding().runner().as_bytes(),
                Arc::new(self.work.clone()),
            )
            .await
            .map_err(backend)?;
        self.owner.check_current()?;
        let decoded = self
            .owner
            .verifier
            .open(response.signed_status(), self.work.clone())?;
        // The admitted resident copy is small metadata; the original decoded
        // generation and SDK response remain guarded through this publication.
        let result = TrustedProviderBoot {
            resident: Allocated::new(
                BootFacts {
                    owner: self.owner.clone(),
                    descriptor: decoded.value().clone(),
                },
                self.resident.clone(),
            ),
        };
        result.check_current()?;
        self.owner
            .verifier
            .source()
            .publish(|| result)
            .map_err(account)
    }
}
struct BootFacts {
    owner: Arc<AccountProviderBoot>,
    descriptor: VerifiedBootDescriptor,
}
/// No Clone or consuming extraction. Only native bounded public facts escape;
/// verified SDK context and original source stay with this opaque owner.
pub struct TrustedProviderBoot {
    resident: Allocated<BootFacts>,
}
impl TrustedProviderBoot {
    pub fn binding(&self) -> &JournalCatalogBinding {
        self.resident.value().owner.verifier.binding()
    }
    pub fn check_current(&self) -> Result<(), ProviderBootError> {
        let facts = self.resident.value();
        let result = (|| {
            let now = facts.owner.check_current()?;
            facts
                .descriptor
                .descriptor()
                .require_current(
                    i64::try_from(now.as_millis()).map_err(|_| ProviderBootError::Clock)?,
                )
                .map_err(|_| ProviderBootError::Expired)
        })();
        result.map_err(|error| retained(error, self.resident.allocation().clone()))
    }
    pub fn boot_id(&self) -> Result<[u8; 16], ProviderBootError> {
        self.check_current()?;
        Ok(self.resident.value().descriptor.descriptor().boot_id())
    }
    pub fn recipient(&self) -> Result<[u8; 32], ProviderBootError> {
        self.check_current()?;
        Ok(self.resident.value().descriptor.descriptor().recipient())
    }
    /// Seals `secret` to this verified boot; see [`LeaseRequest`].
    pub fn seal_lease(
        &self,
        request: &LeaseRequest<'_>,
        secret: &[u8],
    ) -> Result<SealedLease, ProviderBootError> {
        self.check_current()?;
        let facts = self.resident.value();
        facts
            .owner
            .verifier
            .seal(&facts.descriptor, request, secret)
    }
    pub fn expires(&self) -> Result<Timestamp, ProviderBootError> {
        self.check_current()?;
        let expires = self
            .resident
            .value()
            .descriptor
            .descriptor()
            .window()
            .expires_at_ms;
        Ok(Timestamp::from_millis(
            u64::try_from(expires).map_err(|_| ProviderBootError::Expired)?,
        ))
    }
}
/// What a lease of the provider key is sealed under.
pub struct LeaseRequest<'a> {
    /// The provider the runner's configuration names.
    pub provider: &'a str,
    /// Names the lease of this provider in one chat.
    pub lease_id: [u8; 16],
    /// When the lease should end; a boot that ends earlier shortens it.
    pub expires_ms: i64,
}
/// A provider key sealed to one runner boot. Holds ciphertext only.
pub struct SealedLease {
    context: Vec<u8>,
    capsule: Vec<u8>,
    expires: Timestamp,
}
impl SealedLease {
    pub fn context(&self) -> &[u8] {
        &self.context
    }
    pub fn capsule(&self) -> &[u8] {
        &self.capsule
    }
    pub fn expires(&self) -> Timestamp {
        self.expires
    }
}
impl std::fmt::Debug for SealedLease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SealedLease({} bytes)", self.capsule.len())
    }
}
fn require_resident(value: &AllocationLease) -> Result<(), ProviderBootError> {
    if value.bytes() < PROVIDER_BOOT_RESIDENT_BYTES || value.payloads() == 0 {
        return Err(ProviderBootError::Capacity);
    }
    Ok(())
}
fn require_work(value: &AllocationLease) -> Result<(), ProviderBootError> {
    if value.bytes() < provider_boot_work_bytes() || value.payloads() == 0 {
        return Err(ProviderBootError::Capacity);
    }
    Ok(())
}
pub(in crate::cloud) fn announcement_body(
    signed: &nocterm_hub_model::cloud::provider::SignedBootDescriptor,
    allocation: &AllocationLease,
) -> Result<Vec<u8>, ProviderBootError> {
    require_work(allocation)?;
    signed.canonical_bytes().map_err(backend)
}
fn account(error: HubError) -> ProviderBootError {
    ProviderBootError::Storage(AccountMaterialError::Account(error))
}
fn backend(error: impl Error + Send + Sync + 'static) -> ProviderBootError {
    ProviderBootError::Storage(AccountMaterialError::Backend(SharedCause::new(error)))
}
fn retained(error: ProviderBootError, allocation: AllocationLease) -> ProviderBootError {
    match error {
        ProviderBootError::Storage(cause) => {
            ProviderBootError::Storage(super::account_material::retain_error(cause, allocation))
        }
        other => other,
    }
}
