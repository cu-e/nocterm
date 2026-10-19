//! Safe native current-directory inspection using initialized caller storage.
use super::GetCurrentDirectoryW;
use crate::Win32::Foundation::{GetLastError, ERROR_INVALID_DATA};

/// The native result borrows the original caller buffer or asks for its growth.
#[derive(Debug, PartialEq, Eq)]
pub enum CurrentDirectory<'a> {
    /// Original UTF-16 path, excluding its final NUL.
    Written(&'a [u16]),
    /// Native required capacity, including the final NUL.
    Required(u32),
}

/// Query the current directory without allocating or converting its native text.
///
/// The caller can reserve a replacement buffer after `Required` and retry. The
/// directory may change between calls, so a retry can ask for another growth.
/// A failure returns the original `GetLastError` code immediately. No Rust heap
/// allocation, process-wide state change, path limit or normalization is added.
pub fn current_directory_into(buffer: &mut [u16]) -> Result<CurrentDirectory<'_>, u32> {
    let capacity = buffer.len().min(u32::MAX as usize) as u32;
    // SAFETY: the initialized slice provides at least `capacity` writable u16s.
    // A zero capacity still passes the original non-null caller pointer and the
    // native API does not dereference it while reporting required capacity.
    let length = unsafe { GetCurrentDirectoryW(capacity, buffer.as_mut_ptr()) };
    if length == 0 {
        // SAFETY: GetLastError has no pointer arguments. Capture immediately,
        // before any intervening native operation could alter the error code.
        return Err(unsafe { GetLastError() });
    }
    if length >= capacity {
        return Ok(CurrentDirectory::Required(length));
    }
    if buffer[length as usize] != 0 {
        // The native successful-result contract promises this terminal NUL.
        return Err(ERROR_INVALID_DATA);
    }
    Ok(CurrentDirectory::Written(&buffer[..length as usize]))
}

#[cfg(all(test, windows))]
mod tests {
    extern crate std;
    use super::*;
    use std::os::windows::ffi::OsStrExt;

    #[test]
    fn current_directory_borrows_original_buffer_and_matches_std_native_text() {
        let expected = std::env::current_dir().unwrap();
        let expected: std::vec::Vec<_> = expected.as_os_str().encode_wide().collect();
        let mut storage = std::vec![0u16; expected.len() + 1];
        let pointer = storage.as_ptr();
        let CurrentDirectory::Written(actual) = current_directory_into(&mut storage).unwrap() else {
            panic!("unchanged current directory must fit");
        };
        assert_eq!(actual, expected);
        assert_eq!(actual.as_ptr(), pointer);
    }

    #[test]
    fn current_directory_reports_native_growth_without_allocating() {
        let mut empty = [];
        let CurrentDirectory::Required(required) = current_directory_into(&mut empty).unwrap() else {
            panic!("empty storage requires native growth");
        };
        let mut storage = std::vec![0u16; required as usize];
        assert!(matches!(current_directory_into(&mut storage).unwrap(), CurrentDirectory::Written(_)));
    }
}
