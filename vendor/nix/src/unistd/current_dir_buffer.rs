//! Nocterm addition: preserve libc getcwd semantics with caller-owned storage.
use crate::{errno::Errno, Result};
use std::ffi::CStr;

/// Return the current directory in the supplied initialized byte buffer.
///
/// Uses the same native `getcwd` as [`super::getcwd`], without allocating or
/// growing a Rust buffer. A successful result borrows the caller's buffer.
/// Callers may retry `ERANGE` with a larger buffer; no path/depth limit is added.
/// Native failures, including an unlinked current directory, retain their errno.
/// libc-internal C allocations are outside this Rust-buffer contract.
pub fn getcwd_into(buffer: &mut [u8]) -> Result<&CStr> {
    // SAFETY: the non-null pointer denotes buffer.len() writable initialized
    // bytes. libc getcwd writes at most that length, including its final NUL;
    // a zero-length buffer is passed unchanged for the native error result.
    let result = unsafe { libc::getcwd(buffer.as_mut_ptr().cast(), buffer.len()) };
    if result.is_null() {
        return Err(Errno::last());
    }
    // Safe validation keeps the borrowed result within the supplied slice even
    // if a platform violates the native success/NUL postcondition.
    CStr::from_bytes_until_nul(buffer).map_err(|_| Errno::EIO)
}
