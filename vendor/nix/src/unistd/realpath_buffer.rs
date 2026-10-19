//! Nocterm addition: retain realpath's original C owner until a paid Rust copy.
use crate::{errno::Errno, Result};
use std::{ffi::CStr, ptr::NonNull};
/// The original libc-owned canonical path, without an intermediate Rust buffer.
///
/// This is deliberately not a Rust allocator bound: libc's allocation is native
/// C storage, always released with the same libc `free`. The borrowed bytes let
/// a caller admit its exact Rust path backing before copying. No raw ownership
/// escapes, fixed PATH_MAX limit or alternate canonicalization algorithm is used.
#[derive(Debug)]
pub struct RealPath {
    pointer: NonNull<libc::c_char>,
}
impl RealPath {
    /// Borrow the native path for as long as this original owner is alive.
    pub fn as_c_str(&self) -> &CStr {
        // SAFETY: realpath(NULL) returns a fresh NUL-terminated allocation. The
        // private pointer remains owned and unchanged until this object's Drop.
        unsafe { CStr::from_ptr(self.pointer.as_ptr()) }
    }
}
impl Drop for RealPath {
    fn drop(&mut self) {
        // SAFETY: this pointer came from libc realpath(NULL), has not escaped,
        // and is freed once with libc free after all borrowed references end.
        unsafe { libc::free(self.pointer.as_ptr().cast()) }
    }
}
/// Resolve the borrowed native path using libc's allocating realpath operation.
///
/// The caller owns its input C string; this facade creates no Rust path/string
/// allocation and preserves errno on failure. The native output has no assumed
/// capacity and remains alive through a caller's admission refusal or unwind.
pub fn realpath_owned(path: &CStr) -> Result<RealPath> {
    // SAFETY: path is a valid terminated C string. NULL requests a libc-owned
    // result buffer sized by realpath; only the RAII owner below receives it.
    let pointer = unsafe { libc::realpath(path.as_ptr(), std::ptr::null_mut()) };
    NonNull::new(pointer)
        .map(|pointer| RealPath { pointer })
        .ok_or_else(Errno::last)
}
