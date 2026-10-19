//! Nocterm addition: a whole native environment observation in caller storage.
use crate::{errno::Errno, Result};
use std::ffi::CStr;

/// Exact native raw entry count and NUL-separated output storage requirements.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EnvironmentSnapshot {
    /// The output is too small; nothing was written.
    Required {
        /// Bytes of all entries, each including its NUL.
        bytes: usize,
        /// Number of raw entries.
        entries: usize,
    },
    /// Every entry was copied into the start of the output.
    Copied {
        /// Bytes written, each entry including its NUL.
        bytes: usize,
        /// Number of raw entries written.
        entries: usize,
    },
}

/// Copy the current complete native environment, preserving raw entry order,
/// duplicates, empty values and bytes. Every copied entry includes its NUL.
///
/// `Required` leaves the caller's output untouched. No borrowed environment
/// pointer, callback or lock escapes. Environment mutation must exclude native
/// readers according to the mutator's safety obligations, including the exact
/// Unix `std::env::set_var`/`remove_var` requirement for reads outside std::env.
/// This read-only operation neither takes nor pretends to take std's private lock.
pub fn environment_into(output: &mut [u8]) -> Result<EnvironmentSnapshot> {
    let table = environment_table()?;
    let (mut bytes, mut entries) = (0usize, 0usize);
    if !table.is_null() {
        let mut current = table;
        // SAFETY: native environment table is NUL terminated, and each non-null
        // entry is a terminated native string. Unsafe environment mutators must
        // exclude this entire read, so the sizing and copy passes see one table.
        unsafe {
            while !(*current).is_null() {
                let entry = CStr::from_ptr(*current).to_bytes_with_nul();
                bytes = bytes.checked_add(entry.len()).ok_or(Errno::EOVERFLOW)?;
                entries = entries.checked_add(1).ok_or(Errno::EOVERFLOW)?;
                current = current.add(1);
            }
        }
    }
    if output.len() < bytes {
        return Ok(EnvironmentSnapshot::Required { bytes, entries });
    }
    let mut offset = 0;
    let mut current = table;
    for _ in 0..entries {
        // SAFETY: same table and caller-buffer bounds measured above. The entry
        // is borrowed only for this copy; no pointer/reference escapes the call.
        let entry = unsafe { CStr::from_ptr(*current).to_bytes_with_nul() };
        output[offset..offset + entry.len()].copy_from_slice(entry);
        offset += entry.len();
        // SAFETY: the original counted table has another slot (or terminator).
        current = unsafe { current.add(1) };
    }
    Ok(EnvironmentSnapshot::Copied { bytes, entries })
}

#[cfg(target_vendor = "apple")]
fn environment_table() -> Result<*mut *mut libc::c_char> {
    // SAFETY: documented native environment pointer accessor, just as pinned
    // std's sys/env/unix.rs Apple path. Native mutation exclusion applies above.
    let slot = unsafe { libc::_NSGetEnviron() };
    if slot.is_null() {
        return Err(Errno::EIO);
    }
    // SAFETY: accessor returned the native pointer slot.
    Ok(unsafe { *slot })
}
#[cfg(target_os = "freebsd")]
fn environment_table() -> Result<*mut *mut libc::c_char> {
    // FreeBSD's environ lives in CRT, not libc. Resolve the same symbol as std
    // on each call; do not add a cached global pointer or Rust allocation.
    let name = CStr::from_bytes_with_nul(b"environ\0").expect("static environ symbol");
    // SAFETY: constant NUL-terminated symbol and documented RTLD_DEFAULT handle.
    let slot = unsafe { libc::dlsym(libc::RTLD_DEFAULT, name.as_ptr()) };
    if slot.is_null() {
        return Err(Errno::ENOENT);
    }
    // SAFETY: environ is the native CRT environment pointer variable.
    Ok(unsafe { *slot.cast::<*mut *mut libc::c_char>() })
}
#[cfg(not(any(target_vendor = "apple", target_os = "freebsd")))]
fn environment_table() -> Result<*mut *mut libc::c_char> {
    extern "C" {
        static mut environ: *mut *mut libc::c_char;
    }
    // SAFETY: read the POSIX native table; no mutable reference is constructed.
    // Unsafe environment mutation must exclude the entire observation above.
    Ok(unsafe { environ })
}
