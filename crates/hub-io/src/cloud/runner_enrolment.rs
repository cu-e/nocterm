//! Imports the runner a pool host prepared for a managed agent and mints its grant ticket.
//!
//! The setup document arrives from the hub, so the caller must have verified the host's
//! attestation under the pool keys pinned on this device before calling [`enrol`]; that
//! verification replaces the person's out-of-band comparison of the document. The hub-trust
//! record, the account material and the runner registry are the ordinary account owners; the
//! first managed runner of a hub creates the hub-trust record, later ones must agree with it.

use nocterm_cloud_model::coordinator::IntakePoolLimits;
use nocterm_cloud_model::journal::{
    PublicationAllocation, PublicationBudget, PublicationWorkBudget,
};
use nocterm_core::allocation::AllocationLease;
use nocterm_hub_client::cloud::CloudError;
use nocterm_hubs_model::HubError;
use tokio::runtime::Handle;
use zeroize::Zeroizing;

use super::{
    AccountMaterialError, AccountMaterialReservations, AccountRunnerRegistry, CloudRunnerSetup,
    CloudTrustError, CloudTrustReservations, RunnerRegistration, RunnerRegistryError,
    RunnerRegistryReservations, account_material_work_bytes, cloud_trust_work_bytes,
    runner_catalog_work_bytes, runner_registry_work_bytes,
};
use crate::hub_account::HubAccount;

const RESIDENT_ALLOCATION_BYTES: usize = 16 * 1024;

/// Why a runner could not be imported or given its grant.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum EnrolmentError {
    /// The signed-in account is not available (signed out or replaced meanwhile).
    #[error("the account is not available")]
    Account(#[source] HubError),
    /// The setup document or the hub-trust record refused.
    #[error("the hub trust refused the runner")]
    Trust(#[source] CloudTrustError),
    /// The encrypted account material is not available.
    #[error("the account material is not available")]
    Material(#[source] AccountMaterialError),
    /// The runner registry refused.
    #[error("the runner registry refused the runner")]
    Registry(#[source] RunnerRegistryError),
    /// The hub refused to mint the grant ticket.
    #[error("the hub refused the runner grant")]
    Grant(#[source] CloudError),
    /// A bounded allocation for the work could not be reserved.
    #[error("the work does not fit the allocation budget")]
    Budget,
}

/// Finite pools one enrolment reserves from before it does any work.
struct Budget {
    work: PublicationBudget,
    resident: PublicationBudget,
    work_bytes: usize,
}

impl Budget {
    fn new() -> Result<Self, EnrolmentError> {
        let work_bytes = [
            cloud_trust_work_bytes().map_err(EnrolmentError::Trust)?,
            account_material_work_bytes().map_err(EnrolmentError::Material)?,
            runner_registry_work_bytes().map_err(EnrolmentError::Registry)?,
            runner_catalog_work_bytes(),
        ]
        .into_iter()
        .max()
        .unwrap_or(0);
        let limits = |bytes, count| IntakePoolLimits::new(bytes, count, count);
        let work = limits(work_bytes.saturating_mul(3), 3).map_err(|_| EnrolmentError::Budget)?;
        let resident = limits(1024 * 1024, 24).map_err(|_| EnrolmentError::Budget)?;
        Ok(Self {
            work: PublicationBudget::maintenance(work),
            resident: PublicationBudget::maintenance(resident),
            work_bytes,
        })
    }

    fn reserve(pool: &PublicationBudget, bytes: usize) -> Result<AllocationLease, EnrolmentError> {
        let request = PublicationAllocation::new(bytes, 1).map_err(|_| EnrolmentError::Budget)?;
        pool.try_reserve(request)
            .map_err(|_| EnrolmentError::Budget)
    }

    fn resident(&self) -> Result<AllocationLease, EnrolmentError> {
        Self::reserve(&self.resident, RESIDENT_ALLOCATION_BYTES)
    }

    fn work(&self) -> Result<AllocationLease, EnrolmentError> {
        Self::reserve(&self.work, self.work_bytes)
    }
}

/// Imports the runner described by `setup_document` into the account of `account` and returns
/// a short-lived ticket the host redeems with the hub.
pub async fn enrol(
    account: &HubAccount,
    handle: Handle,
    setup_document: &[u8],
    label: String,
) -> Result<Zeroizing<String>, EnrolmentError> {
    let budget = Budget::new()?;
    let setup = CloudRunnerSetup::from_public_toml(setup_document, label.clone())
        .map_err(EnrolmentError::Trust)?;
    let registration = RunnerRegistration::new(
        setup.runner().binding().clone(),
        setup.runner().pins(),
        label,
    )
    .map_err(EnrolmentError::Registry)?;
    let runner = *registration.binding().runner().as_bytes();
    let proof = registration.pins().proof;

    let source = account.local_source().map_err(EnrolmentError::Account)?;
    let trust = account
        .cloud_trust(source.clone(), budget.resident()?)
        .map_err(EnrolmentError::Trust)?;
    let reservation = || {
        CloudTrustReservations::new(budget.work()?, budget.resident()?)
            .map_err(EnrolmentError::Trust)
    };
    let trusted = match trust.load(reservation()?).await {
        Ok(trusted) => trusted,
        Err(CloudTrustError::NotConfigured) => trust
            .register_new(setup, reservation()?)
            .await
            .map_err(EnrolmentError::Trust)?,
        Err(error) => return Err(EnrolmentError::Trust(error)),
    };
    let material_reservations =
        AccountMaterialReservations::new(budget.work()?, budget.resident()?)
            .map_err(EnrolmentError::Material)?;
    let material = account
        .cloud_material(source, trusted.authority(), material_reservations)
        .await
        .map_err(EnrolmentError::Material)?;
    let registry = AccountRunnerRegistry::capture(
        &material,
        trusted.scope().clone(),
        budget.resident()?,
        handle,
    )
    .map_err(EnrolmentError::Registry)?;
    let registry_reservations = || {
        RunnerRegistryReservations::new(budget.work()?, budget.resident()?)
            .map_err(EnrolmentError::Registry)
    };
    let roster = registry
        .load(registry_reservations()?)
        .await
        .map_err(EnrolmentError::Registry)?;
    let known = roster
        .runners()
        .map_err(EnrolmentError::Registry)?
        .iter()
        .any(|row| row.binding() == registration.binding());
    if !known {
        registry
            .register_new(registration, registry_reservations()?)
            .await
            .map_err(EnrolmentError::Registry)?;
    }
    let (_, cloud, _) = material.conversation_parts();
    let ticket = cloud
        .create_runner_grant(runner, proof)
        .await
        .map_err(EnrolmentError::Grant)?;
    Ok(Zeroizing::new(ticket.token().to_owned()))
}
