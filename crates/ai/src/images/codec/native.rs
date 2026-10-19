//! Borrowed native image allocation bank and preadmitted terminal error owner.
pub use crate::model::failure::{
    ManagedNativeFailure as ManagedImageCause, NativeErrorSlot as NativeImageErrorSlot,
};
use crate::model::json::deferred::{DecodeLedger, NativeAllocationError, ledger::Reservation};
use std::sync::Mutex;

#[derive(Clone, Copy, Debug, thiserror::Error)]
#[error("native image allocation refused")]
pub struct NativeImageAllocationRefused;
#[derive(Debug, thiserror::Error)]
pub enum NativeImageScopeError {
    #[error(transparent)]
    Allocation(#[from] NativeAllocationError),
    #[error("native image allocation owner is poisoned")]
    Poisoned,
    #[error("native image allocation owners did not retire {bytes} bytes")]
    Outstanding { bytes: usize },
}
struct AllocationState {
    bank: Reservation,
    failure: Option<NativeAllocationError>,
}
/// Borrowed by decoder-owned inline receipts. Native owners call retirement
/// only after freeing their backing; consuming this scope requires those borrows
/// to end, including destructors of the original decoder and output buffers.
pub struct NativeImageAllocationScope<'a> {
    state: Mutex<AllocationState>,
    _ledger: &'a DecodeLedger,
}
impl<'a> NativeImageAllocationScope<'a> {
    pub fn begin(ledger: &'a DecodeLedger) -> Result<Self, NativeAllocationError> {
        Ok(Self {
            state: Mutex::new(AllocationState {
                bank: ledger.reserve_requested(0)?,
                failure: None,
            }),
            _ledger: ledger,
        })
    }
    /// Call before the concrete native allocator. The first refusal is sticky,
    /// including when optional native metadata parsing ignores its own error.
    pub fn admit_before_native_allocation(
        &self,
        bytes: usize,
    ) -> Result<(), NativeImageAllocationRefused> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| NativeImageAllocationRefused)?;
        if state.failure.is_some() {
            return Err(NativeImageAllocationRefused);
        }
        if let Err(error) = state.bank.grow_requested(bytes) {
            state.failure = Some(error);
            return Err(NativeImageAllocationRefused);
        }
        Ok(())
    }
    /// Used exclusively by actual native backing owners after physical free.
    pub fn retire_after_native_drop(&self, bytes: usize) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let retired = state
            .bank
            .split(bytes)
            .expect("original native image backing receipt");
        drop(retired);
    }
    pub fn finish(self) -> Result<(), NativeImageScopeError> {
        let state = self
            .state
            .into_inner()
            .map_err(|_| NativeImageScopeError::Poisoned)?;
        if state.bank.bytes() != 0 {
            return Err(NativeImageScopeError::Outstanding {
                bytes: state.bank.bytes(),
            });
        }
        if let Some(error) = state.failure {
            return Err(error.into());
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests;
