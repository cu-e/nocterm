//! Native phase scratch is paid before the adapter creates its actual buffer.
//! This token carries no payload identity and cannot become a census component.
use super::{DecodeLedger, JsonDecodeError, ledger::Reservation};

#[must_use = "keep the phase token until its actual scratch allocations retire"]
pub struct NativeScratch {
    allocation: Reservation,
}
impl NativeScratch {
    pub(crate) fn release_unused(&mut self, bytes: usize) {
        drop(
            self.allocation
                .split(bytes)
                .expect("unused preadmitted native error bytes"),
        );
    }
    pub fn reserve_native(
        ledger: &DecodeLedger,
        bytes: usize,
    ) -> Result<Self, super::NativeAllocationError> {
        Ok(Self {
            allocation: ledger.reserve_requested(bytes)?,
        })
    }
    /// The adapter derives checked layout/capacity from its concrete scratch
    /// owner. A reservation is not evidence that the buffer was allocated.
    pub fn reserve(ledger: &DecodeLedger, bytes: usize) -> Result<Self, JsonDecodeError> {
        Ok(Self {
            allocation: ledger.reserve(bytes)?,
        })
    }
    /// Call before growing the actual buffer; failure leaves prior credit held.
    pub fn grow_before_allocation(&mut self, additional: usize) -> Result<(), JsonDecodeError> {
        self.allocation.grow(additional)
    }
    pub fn requested_bytes(&self) -> usize {
        self.allocation.bytes()
    }
}
