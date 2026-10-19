//! Common preadmitted native error ownership keeps exact concrete causes alive.
use crate::model::json::deferred::{DecodeLedger, NativeAllocationError, NativeScratch};
use std::{error::Error, fmt, marker::PhantomData};
struct NativeCause {
    original: Box<dyn Error + Send + Sync>,
    _allocation: NativeScratch,
}
impl fmt::Debug for NativeCause {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.original, f)
    }
}
impl fmt::Display for NativeCause {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.original, f)
    }
}
impl Error for NativeCause {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.original.as_ref())
    }
}
/// The original typed native cause and its exact paid Box/shared header remain
/// inseparable. Cloning preserves the same concrete cause and physical credit.
#[derive(Clone, Debug)]
pub struct ManagedNativeFailure(nocterm_core::SharedResidentError<NativeCause, NativeScratch>);
impl fmt::Display for ManagedNativeFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}
impl Error for ManagedNativeFailure {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.0.as_ref().original.as_ref())
    }
}
/// Prepared before native constructors. Nested bytes are derived by the native
/// adapter from the concrete error path's actual additional owned Box layouts.
pub struct NativeErrorSlot<E: Error + Send + Sync + 'static> {
    body: NativeScratch,
    payload: NativeScratch,
    nested: usize,
    _error: PhantomData<E>,
}
impl<E: Error + Send + Sync + 'static> NativeErrorSlot<E> {
    pub fn prepare(
        ledger: &DecodeLedger,
        nested_error_bytes: usize,
    ) -> Result<Self, NativeAllocationError> {
        let body = NativeScratch::reserve_native(
            ledger,
            nocterm_core::SharedResidentError::<NativeCause, NativeScratch>::allocation_bytes(),
        )?;
        let bytes = std::mem::size_of::<E>()
            .checked_add(nested_error_bytes)
            .ok_or(NativeAllocationError::Arithmetic)?;
        let payload = NativeScratch::reserve_native(ledger, bytes)?;
        Ok(Self {
            body,
            payload,
            nested: nested_error_bytes,
            _error: PhantomData,
        })
    }
    pub fn capture(self, error: E) -> ManagedNativeFailure {
        let nested = self.nested;
        self.capture_with_nested(error, nested)
    }
    /// The closed native error branch supplies its actual nested allocation.
    /// Unused prospective bytes have no backing and retire before publication.
    pub fn capture_with_nested(self, error: E, actual_nested_bytes: usize) -> ManagedNativeFailure {
        let mut capture = ErrorCapture { error, slot: self };
        let unused = capture
            .slot
            .nested
            .checked_sub(actual_nested_bytes)
            .expect("native error matches its prepared layout");
        capture.slot.payload.release_unused(unused);
        let original = Box::new(capture.error);
        ManagedNativeFailure(nocterm_core::SharedResidentError::new(
            NativeCause {
                original,
                _allocation: capture.slot.payload,
            },
            capture.slot.body,
        ))
    }
}

// If a concrete native layout invariant panics, destroy its actual error and
// nested allocations before the still-preadmitted ticket can retire.
struct ErrorCapture<E: Error + Send + Sync + 'static> {
    error: E,
    slot: NativeErrorSlot<E>,
}
