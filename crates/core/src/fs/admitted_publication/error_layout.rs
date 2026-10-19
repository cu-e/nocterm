//! Pinned Rust1.98.0 io::Error custom body requested allocation layout.
use std::{error::Error, io::ErrorKind, ptr::NonNull};

// core/src/io/error.rs::Custom (repr align4); alloc/src/io/error.rs allocates
// this body separately from the original boxed concrete error. No execution or
// raw allocation address is exposed; independent allocator tests must bind this
// source-derived layout to the actual pinned toolchain's requests.
#[repr(align(4))]
struct IoCustomLayout {
    _kind: ErrorKind,
    _error: NonNull<dyn Error + Send + Sync>,
    _error_drop: unsafe fn(*mut (dyn Error + Send + Sync)),
    _outer_drop: unsafe fn(*mut Self),
}

/// Requested bytes of the pinned std custom io::Error body, excluding its cause.
/// The original concrete cause's own allocation/backings must be admitted too.
pub fn io_error_custom_allocation_bytes() -> usize {
    std::mem::size_of::<IoCustomLayout>()
}
