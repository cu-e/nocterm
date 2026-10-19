//! Nocterm addition: the pinned std home fallback with caller-owned scratch.
use crate::{errno::Errno, Result};
use std::{ffi::CStr, mem::MaybeUninit, ptr};

/// Original std1.98 passwd scratch choice; this does not add retry or a size cap.
pub fn passwd_home_buffer_size() -> usize {
    // SAFETY: sysconf takes a constant selector and owns no caller memory.
    match unsafe { libc::sysconf(libc::_SC_GETPW_R_SIZE_MAX) } {
        value if value < 0 => 512,
        value => value as usize,
    }
}

/// Observe the current uid's passwd home using exactly one native lookup.
///
/// The returned bytes borrow the supplied buffer. Unlike User::from_uid, this
/// helper does not retry ERANGE or substitute a different scratch policy; callers
/// preserving std::env::home_dir map native lookup failures to missing home.
pub fn passwd_home_into(output: &mut [u8]) -> Result<Option<&[u8]>> {
    let mut entry = MaybeUninit::<libc::passwd>::uninit();
    let mut result = ptr::null_mut();
    // SAFETY: entry/result are writable outputs, and the initialized byte slice
    // supplies exactly its declared writable scratch length. getuid is read-only.
    let code = unsafe {
        libc::getpwuid_r(
            libc::getuid(),
            entry.as_mut_ptr(),
            output.as_mut_ptr().cast(),
            output.len(),
            &mut result,
        )
    };
    if code != 0 {
        return Err(Errno::from_raw(code));
    }
    if result.is_null() {
        return Ok(None);
    }
    // SAFETY: a successful non-null result initialized entry. POSIX requires its
    // string fields in the supplied buffer; validate bounds before borrowing.
    let pointer = unsafe { entry.assume_init() }.pw_dir;
    let start = (pointer as usize)
        .checked_sub(output.as_ptr() as usize)
        .filter(|start| *start < output.len())
        .ok_or(Errno::EIO)?;
    let value = CStr::from_bytes_until_nul(&output[start..]).map_err(|_| Errno::EIO)?;
    Ok(Some(value.to_bytes()))
}
