//! The finite pools every Cloud chat operation reserves from before it does any work.

use nocterm_cloud_model::coordinator::IntakePoolLimits;
use nocterm_cloud_model::journal::{
    PublicationAllocation, PublicationBudget, PublicationWorkBudget,
};
use nocterm_core::allocation::AllocationLease;

use super::error::CloudChatError;
use crate::cloud::{
    ACCOUNT_MATERIAL_RESIDENT_BYTES, AccountMaterialReservations, CLOUD_TRUST_RESIDENT_BYTES,
    CloudConversationSource, CloudTrustReservations, RUNNER_CATALOG_RESIDENT_BYTES,
    RUNNER_REGISTRY_RESIDENT_BYTES, RunnerRegistryReservations, account_material_work_bytes,
    cloud_trust_work_bytes, runner_catalog_page_bytes, runner_catalog_work_bytes,
    runner_registry_work_bytes,
};

/// Resident bytes a live conversation source keeps for as long as a window shows it.
pub(super) const SOURCE_RESIDENT_BYTES: usize = 64 * 1024;
const RESIDENT_POOL_BYTES: usize = 32 * 1024 * 1024;
const RESIDENT_POOL_PAYLOADS: usize = 1024;
const WORK_POOL_PAYLOADS: usize = 8;

/// Work and resident pools shared by the chats of one installation.
pub(super) struct ChatBudget {
    work: PublicationBudget,
    resident: PublicationBudget,
}

impl ChatBudget {
    pub(super) fn new() -> Result<Self, CloudChatError> {
        let largest = Self::capture_work_bytes()?
            .max(Self::trust_work_bytes()?)
            .max(account_material_work_bytes().map_err(|_| CloudChatError::Capacity)?)
            .max(runner_registry_work_bytes().map_err(|_| CloudChatError::Capacity)?)
            .max(runner_catalog_work_bytes());
        let work = IntakePoolLimits::new(
            largest.saturating_mul(WORK_POOL_PAYLOADS),
            WORK_POOL_PAYLOADS,
            WORK_POOL_PAYLOADS,
        )
        .map_err(|_| CloudChatError::Capacity)?;
        let resident = IntakePoolLimits::new(
            RESIDENT_POOL_BYTES,
            RESIDENT_POOL_PAYLOADS,
            RESIDENT_POOL_PAYLOADS,
        )
        .map_err(|_| CloudChatError::Capacity)?;
        Ok(Self {
            work: PublicationBudget::maintenance(work),
            resident: PublicationBudget::maintenance(resident),
        })
    }

    fn trust_work_bytes() -> Result<usize, CloudChatError> {
        cloud_trust_work_bytes().map_err(|_| CloudChatError::Capacity)
    }

    pub(super) fn capture_work_bytes() -> Result<usize, CloudChatError> {
        CloudConversationSource::capture_work_bytes().map_err(|_| CloudChatError::Capacity)
    }

    fn reserve(pool: &PublicationBudget, bytes: usize) -> Result<AllocationLease, CloudChatError> {
        let request = PublicationAllocation::new(bytes, 1).map_err(|_| CloudChatError::Capacity)?;
        pool.try_reserve(request)
            .map_err(|_| CloudChatError::Capacity)
    }

    pub(super) fn work(&self, bytes: usize) -> Result<AllocationLease, CloudChatError> {
        Self::reserve(&self.work, bytes)
    }

    pub(super) fn resident(&self, bytes: usize) -> Result<AllocationLease, CloudChatError> {
        Self::reserve(&self.resident, bytes)
    }

    pub(super) fn trust(&self) -> Result<CloudTrustReservations, CloudChatError> {
        CloudTrustReservations::new(
            self.work(Self::trust_work_bytes()?)?,
            self.resident(CLOUD_TRUST_RESIDENT_BYTES)?,
        )
        .map_err(|_| CloudChatError::Capacity)
    }

    pub(super) fn material(&self) -> Result<AccountMaterialReservations, CloudChatError> {
        AccountMaterialReservations::new(
            self.work(account_material_work_bytes().map_err(|_| CloudChatError::Capacity)?)?,
            self.resident(ACCOUNT_MATERIAL_RESIDENT_BYTES)?,
        )
        .map_err(|_| CloudChatError::Capacity)
    }

    pub(super) fn registry(&self) -> Result<RunnerRegistryReservations, CloudChatError> {
        RunnerRegistryReservations::new(
            self.work(runner_registry_work_bytes().map_err(|_| CloudChatError::Capacity)?)?,
            self.resident(RUNNER_REGISTRY_RESIDENT_BYTES)?,
        )
        .map_err(|_| CloudChatError::Capacity)
    }

    pub(super) fn catalog_resident(&self) -> Result<AllocationLease, CloudChatError> {
        self.resident(RUNNER_CATALOG_RESIDENT_BYTES)
    }

    pub(super) fn catalog_work(&self) -> Result<AllocationLease, CloudChatError> {
        self.work(runner_catalog_work_bytes())
    }

    pub(super) fn catalog_page(&self) -> Result<AllocationLease, CloudChatError> {
        self.resident(runner_catalog_page_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pools_fit_a_whole_creation_at_once() {
        let budget = ChatBudget::new().unwrap();
        let _source_work = budget
            .work(ChatBudget::capture_work_bytes().unwrap())
            .unwrap();
        let _source_resident = budget.resident(SOURCE_RESIDENT_BYTES).unwrap();
        assert!(budget.trust().is_ok());
        assert!(budget.material().is_ok());
        assert!(budget.registry().is_ok());
        assert!(budget.catalog_work().is_ok());
        assert!(budget.catalog_page().is_ok());
    }

    #[test]
    fn a_released_reservation_frees_its_share() {
        let budget = ChatBudget::new().unwrap();
        for _ in 0..(WORK_POOL_PAYLOADS * 2) {
            let _lease = budget
                .work(ChatBudget::capture_work_bytes().unwrap())
                .unwrap();
        }
    }
}
