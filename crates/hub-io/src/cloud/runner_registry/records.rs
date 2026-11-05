//! The registry object's layout is the contract's (`nocterm_cloud_device::records::
//! RunnerRegistry`, the same bytes every device reads); this module only converts the
//! desktop's registrations to and from it.
use super::{ChatMaterialPins, RunnerRegistration, RunnerRegistryError};
use nocterm_cloud_device::CloudDeviceError;
use nocterm_cloud_device::records::{
    MAX_TRUSTED_RUNNERS, RUNNER_REGISTRY_KIND, RUNNER_REGISTRY_OBJECT, RunnerPins, RunnerRegistry,
    TrustedRunner,
};
use nocterm_cloud_model::{
    identity::{AccountScope, RunnerId, RunnerKeyEpoch},
    journal::JournalCatalogBinding,
};
use nocterm_hub_model::cloud::RunnerCoordinate;
pub(super) const MAX_PLAINTEXT: usize = 8 * 1024;
pub(super) const MAX_RUNNERS: usize = MAX_TRUSTED_RUNNERS;
pub(super) const KIND: &str = RUNNER_REGISTRY_KIND;
pub(super) const OBJECT: &str = RUNNER_REGISTRY_OBJECT;

fn refused(error: CloudDeviceError) -> RunnerRegistryError {
    match error {
        CloudDeviceError::Capacity => RunnerRegistryError::Capacity,
        CloudDeviceError::Binding => RunnerRegistryError::Binding,
        _ => RunnerRegistryError::Format,
    }
}

/// The scope as the contract's records name it.
pub(super) fn device_scope(
    scope: &AccountScope,
) -> Result<nocterm_cloud_device::AccountScope, RunnerRegistryError> {
    nocterm_cloud_device::AccountScope::from_parts(
        scope.hub().instance().as_str(),
        *scope.hub().authority().as_bytes(),
        scope.account().id().as_str(),
        *scope.account().key().as_bytes(),
    )
    .map_err(refused)
}
pub(super) fn collection_id(scope: &AccountScope) -> Result<String, RunnerRegistryError> {
    RunnerRegistry::collection_id(&device_scope(scope)?).map_err(refused)
}
fn trusted(row: &RunnerRegistration) -> Result<TrustedRunner, RunnerRegistryError> {
    TrustedRunner::new(
        RunnerCoordinate {
            id: *row.binding.runner().as_bytes(),
            key_epoch: row.binding.epoch().get(),
        },
        RunnerPins {
            encryption: row.pins.encryption,
            signing: row.pins.signing,
            proof: row.pins.proof,
        },
        &row.label,
    )
    .map_err(refused)
}
pub(super) fn encode(
    scope: &AccountScope,
    version: u64,
    rows: &[RunnerRegistration],
) -> Result<zeroize::Zeroizing<Vec<u8>>, RunnerRegistryError> {
    if version != rows.len() as u64 {
        return Err(RunnerRegistryError::Format);
    }
    let rows = rows.iter().map(trusted).collect::<Result<Vec<_>, _>>()?;
    RunnerRegistry::new(device_scope(scope)?, rows)
        .and_then(|registry| registry.encode())
        .map_err(refused)
}
pub(super) fn decode(
    scope: &AccountScope,
    version: u64,
    bytes: &[u8],
) -> Result<Vec<RunnerRegistration>, RunnerRegistryError> {
    let registry =
        RunnerRegistry::decode(&device_scope(scope)?, version, bytes).map_err(refused)?;
    registry
        .runners()
        .iter()
        .map(|row| {
            let runner = row.runner();
            let pins = row.pins();
            RunnerRegistration::new(
                JournalCatalogBinding::new(
                    scope.clone(),
                    RunnerId::from_bytes(runner.id),
                    RunnerKeyEpoch::new(runner.key_epoch)
                        .map_err(|_| RunnerRegistryError::Format)?,
                ),
                ChatMaterialPins {
                    encryption: pins.encryption,
                    signing: pins.signing,
                    proof: pins.proof,
                },
                row.label().to_owned(),
            )
        })
        .collect()
}
