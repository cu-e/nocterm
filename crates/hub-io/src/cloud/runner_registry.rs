//! One bounded encrypted personal roster; no directory-offered runner key is trusted.
mod operation;
mod records;
mod storage;
#[cfg(test)]
mod tests;
use super::{AccountMaterialError, ChatMaterialPins, CloudAccountMaterial};
use crate::account::operations::BoundOperations;
use futures::future::BoxFuture;
use nocterm_cloud_model::{identity::AccountScope, journal::JournalCatalogBinding};
use nocterm_core::{
    SharedCause,
    allocation::{Allocated, AllocationLease},
};
use nocterm_hub_client::cloud::CloudClient;
use nocterm_hubs_model::HubError;
use sha2::{Digest, Sha256};
use std::{error::Error, sync::Arc};
use tokio::runtime::Handle;

pub const RUNNER_REGISTRY_RESIDENT_BYTES: usize = 16 * 1024;
const MAX_RESPONSE: usize = 16 * 1024;
/// Worst empty repeated Collection expansion plus simultaneous native roster,
/// plaintext, AEAD and original request generations. SDK transport/proof work is
/// separate below; connection/TLS/runtime and caller row copies are not RSS here.
pub fn runner_registry_work_bytes() -> Result<usize, RunnerRegistryError> {
    let collections = (MAX_RESPONSE + 2) * std::mem::size_of::<nocterm_hub_proto::v1::Collection>();
    let native = 2 * RUNNER_REGISTRY_RESIDENT_BYTES;
    let plaintext = 2 * records::MAX_PLAINTEXT;
    let crypto = 2 * (records::MAX_PLAINTEXT + 42);
    let request_and_response = 2 * MAX_RESPONSE;
    Ok(collections
        + native
        + plaintext
        + crypto
        + request_and_response
        + nocterm_hub_client::store_work_bytes(MAX_RESPONSE).map_err(backend)?)
}
#[derive(Debug, thiserror::Error)]
pub enum RunnerRegistryError {
    #[error("the runner registry does not match the captured account or pins")]
    Binding,
    #[error("the runner registry exceeds its finite format or allocation bound")]
    Capacity,
    #[error("the encrypted runner registry is malformed")]
    Format,
    #[error("the runner registry was deleted")]
    Deleted,
    #[error("the roster changed; re-read before an explicit registration retry")]
    Conflict,
    #[error("changing registered runner keys, epoch or name requires an explicit supported action")]
    Rotation,
    #[error("the original account runner registry source is unavailable")]
    Stale,
    #[error("the native runner registry operation failed")]
    Storage(#[source] AccountMaterialError),
}
/// Independently verified setup values. Construction does not authenticate them;
/// registration is an explicit action under the captured account's private key.
#[derive(Debug, PartialEq, Eq)]
pub struct RunnerRegistration {
    binding: JournalCatalogBinding,
    pins: ChatMaterialPins,
    label: String,
}
impl RunnerRegistration {
    pub fn new(
        binding: JournalCatalogBinding,
        pins: ChatMaterialPins,
        label: String,
    ) -> Result<Self, RunnerRegistryError> {
        if label.is_empty() || label.len() > 128 || label.chars().any(char::is_control) {
            return Err(RunnerRegistryError::Capacity);
        }
        Ok(Self {
            binding,
            pins,
            label,
        })
    }
    pub fn binding(&self) -> &JournalCatalogBinding {
        &self.binding
    }
    pub fn pins(&self) -> ChatMaterialPins {
        self.pins
    }
    pub fn label(&self) -> &str {
        &self.label
    }
}
/// Both permits are supplied by actual finite caller-owned pools, before work.
pub struct RunnerRegistryReservations {
    work: AllocationLease,
    resident: AllocationLease,
}
impl RunnerRegistryReservations {
    pub fn new(
        work: AllocationLease,
        resident: AllocationLease,
    ) -> Result<Self, RunnerRegistryError> {
        if work.bytes() < runner_registry_work_bytes()?
            || work.payloads() == 0
            || !resident_fits(&resident)
        {
            return Err(RunnerRegistryError::Capacity);
        }
        Ok(Self { work, resident })
    }
}
pub struct AccountRunnerRegistry {
    bound: BoundOperations,
    cloud: Arc<CloudClient>,
    scope: AccountScope,
    handle: Handle,
    _allocation: AllocationLease,
}
impl AccountRunnerRegistry {
    pub fn capture(
        material: &CloudAccountMaterial,
        scope: AccountScope,
        resident: AllocationLease,
        handle: Handle,
    ) -> Result<Self, RunnerRegistryError> {
        if !resident_fits(&resident) {
            return Err(RunnerRegistryError::Capacity);
        }
        let result = (|| {
            let (bound, cloud, authority) = material.conversation_parts();
            validate_scope(&bound, &cloud, authority, &scope)?;
            Ok(Self {
                bound,
                cloud,
                scope,
                handle,
                _allocation: resident.clone(),
            })
        })();
        result.map_err(|error| retained(error, resident))
    }
    pub fn load(
        &self,
        reservations: RunnerRegistryReservations,
    ) -> BoxFuture<'static, Result<TrustedRunnerRoster, RunnerRegistryError>> {
        self.run(None, reservations)
    }
    pub fn register_new(
        &self,
        runner: RunnerRegistration,
        reservations: RunnerRegistryReservations,
    ) -> BoxFuture<'static, Result<TrustedRunnerRoster, RunnerRegistryError>> {
        self.run(Some(runner), reservations)
    }
    fn run(
        &self,
        registration: Option<RunnerRegistration>,
        reservations: RunnerRegistryReservations,
    ) -> BoxFuture<'static, Result<TrustedRunnerRoster, RunnerRegistryError>> {
        let outer_work = reservations.work.clone();
        let prepared = (|| {
            self.cloud.check_source().map_err(backend)?;
            self.bound.source().check().map_err(account)?;
            if registration
                .as_ref()
                .is_some_and(|row| row.binding.scope() != &self.scope)
            {
                return Err(RunnerRegistryError::Binding);
            }
            let work = self
                .bound
                .capture_bounded(MAX_RESPONSE, reservations.work.clone())
                .map_err(account)?;
            Ok(operation::RegistryOperation::new(
                work,
                self.bound.clone(),
                self.cloud.clone(),
                self.scope.clone(),
                registration,
                reservations.resident,
            ))
        })();
        let operation = match prepared {
            Ok(value) => Allocated::new(value, reservations.work.clone()),
            Err(error) => return Box::pin(async move { Err(retained(error, reservations.work)) }),
        };
        let bound = self.bound.clone();
        let worker = crate::task::spawn(&self.handle, async move {
            operation
                .value()
                .execute()
                .await
                .map_err(|error| retained(error, operation.allocation().clone()))
        });
        Box::pin(async move {
            let result = bound
                .source()
                .during(worker)
                .await
                .map_err(|error| retained(account(error), outer_work.clone()))?
                .ok_or(RunnerRegistryError::Stale)??;
            result.runners()?;
            let result = bound
                .source()
                .publish(|| result)
                .map_err(|error| retained(account(error), outer_work))?;
            result.runners()?;
            Ok(result)
        })
    }
}
struct RosterData {
    scope: AccountScope,
    version: u64,
    runners: Vec<RunnerRegistration>,
    bound: BoundOperations,
    cloud: Arc<CloudClient>,
}
/// Only public pins/labels survive; collection/account plaintext keys do not.
pub struct TrustedRunnerRoster {
    resident: Allocated<RosterData>,
}
impl TrustedRunnerRoster {
    pub fn scope(&self) -> &AccountScope {
        &self.resident.value().scope
    }
    pub fn version(&self) -> u64 {
        self.resident.value().version
    }
    pub fn runners(&self) -> Result<&[RunnerRegistration], RunnerRegistryError> {
        let data = self.resident.value();
        data.bound
            .source()
            .check()
            .map_err(|error| retained(account(error), self.resident.allocation().clone()))?;
        data.cloud
            .check_source()
            .map_err(|error| retained(backend(error), self.resident.allocation().clone()))?;
        Ok(&data.runners)
    }
}
pub(super) fn validate_scope(
    bound: &BoundOperations,
    cloud: &CloudClient,
    authority: [u8; 32],
    scope: &AccountScope,
) -> Result<(), RunnerRegistryError> {
    bound.source().check().map_err(account)?;
    cloud.check_source().map_err(backend)?;
    let subject = bound.source().native().subject().map_err(account)?;
    if scope.hub().instance().as_str() != cloud.namespace().hub_instance_id.as_str()
        || scope.account().id().as_str() != cloud.namespace().account_id.as_str()
        || *scope.hub().authority().as_bytes() != <[u8; 32]>::from(Sha256::digest(authority))
        || subject.account() != cloud.subject().account().as_str()
        || subject.device() != cloud.subject().device().as_str()
    {
        return Err(RunnerRegistryError::Binding);
    }
    let actual = bound
        .source()
        .native()
        .with_current(|keys| {
            Ok(<[u8; 32]>::from(Sha256::digest(
                keys.keyset().public_key().to_bytes(),
            )))
        })
        .map_err(account)?;
    if actual != *scope.account().key().as_bytes() {
        return Err(RunnerRegistryError::Binding);
    }
    Ok(())
}
fn account(error: HubError) -> RunnerRegistryError {
    RunnerRegistryError::Storage(AccountMaterialError::Account(error))
}
fn backend(error: impl Error + Send + Sync + 'static) -> RunnerRegistryError {
    RunnerRegistryError::Storage(AccountMaterialError::Backend(SharedCause::new(error)))
}
fn retained(error: RunnerRegistryError, allocation: AllocationLease) -> RunnerRegistryError {
    match error {
        RunnerRegistryError::Storage(cause) => {
            RunnerRegistryError::Storage(super::account_material::retain_error(cause, allocation))
        }
        rejected => rejected,
    }
}

fn resident_fits(allocation: &AllocationLease) -> bool {
    // A push into an opened16-entry Vec may retain up to twice its old native
    // element capacity. Each row also owns two bounded namespace strings and a
    // bounded label; RosterData owns one additional full scope.
    let maximum = std::mem::size_of::<RosterData>()
        + records::MAX_RUNNERS
            * (2 * std::mem::size_of::<RunnerRegistration>()
                + 2 * nocterm_cloud_model::identity::MAX_NAMESPACE_BYTES
                + 128)
        + 2 * nocterm_cloud_model::identity::MAX_NAMESPACE_BYTES;
    maximum <= RUNNER_REGISTRY_RESIDENT_BYTES
        && allocation.bytes() >= RUNNER_REGISTRY_RESIDENT_BYTES
        && allocation.payloads() != 0
}
