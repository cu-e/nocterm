//! Distinct actual work and resident permits; no quota or authority is created here.
use super::{ACCOUNT_MATERIAL_RESIDENT_BYTES, AccountMaterialError};
use nocterm_core::allocation::AllocationLease;

/// Both permits must come from real finite caller-owned pools. Work follows the
/// actual SDK bodies and native decoded buffers; resident follows returned data.
pub struct AccountMaterialReservations {
    work: AllocationLease,
    resident: AllocationLease,
}
impl AccountMaterialReservations {
    pub fn new(
        work: AllocationLease,
        resident: AllocationLease,
    ) -> Result<Self, AccountMaterialError> {
        let reservations = Self { work, resident };
        reservations.require(1)?;
        Ok(reservations)
    }
    pub(super) fn require(&self, work_bytes: usize) -> Result<(), AccountMaterialError> {
        if self.work.bytes() < work_bytes
            || self.work.payloads() == 0
            || self.resident.bytes() < ACCOUNT_MATERIAL_RESIDENT_BYTES
            || self.resident.payloads() == 0
        {
            Err(AccountMaterialError::Capacity)
        } else {
            Ok(())
        }
    }
    pub(super) fn work(&self) -> AllocationLease {
        self.work.clone()
    }
    pub(super) fn resident(&self) -> AllocationLease {
        self.resident.clone()
    }
}
