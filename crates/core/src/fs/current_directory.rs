//! Per-operation native cwd queries borrow genuinely admitted caller storage.
use super::PathAdmissionError;
use std::io;
#[cfg(unix)]
use std::path::Path;
#[cfg(unix)]
type NativeUnit = u8;
#[cfg(windows)]
type NativeUnit = u16;

#[derive(Debug, thiserror::Error)]
pub enum CurrentDirectoryError {
    #[error(transparent)]
    Admission(#[from] PathAdmissionError),
    #[error("native current-directory query failed: {0}")]
    Native(#[source] io::Error),
}
/// A result borrowing original source storage, or a native request for its growth.
pub enum DirectoryObservation<'a> {
    #[cfg(unix)]
    Native(&'a Path),
    #[cfg(windows)]
    Wide(&'a [u16]),
    Required(usize),
}
/// Owns initialized native output storage before the actual native query.
/// Storage physically drops before the caller's inseparable allocation credit.
pub struct CurrentDirectoryBuffer<Credit> {
    storage: Vec<NativeUnit>,
    _credit: Credit,
}
impl<Credit> CurrentDirectoryBuffer<Credit> {
    pub fn requested_bytes(capacity: usize) -> Result<usize, PathAdmissionError> {
        capacity
            .checked_mul(std::mem::size_of::<NativeUnit>())
            .ok_or(PathAdmissionError::Arithmetic)
    }
    /// Prepay requested_bytes(capacity), including overlap with any old buffer.
    pub fn new(capacity: usize, credit: Credit) -> Result<Self, PathAdmissionError> {
        Self::requested_bytes(capacity)?;
        let mut storage = Vec::new();
        storage.try_reserve_exact(capacity)?;
        if storage.capacity() != capacity {
            return Err(PathAdmissionError::CapacityProfile);
        }
        storage.resize(capacity, 0);
        Ok(Self {
            storage,
            _credit: credit,
        })
    }
    pub fn capacity(&self) -> usize {
        self.storage.capacity()
    }
    #[cfg(unix)]
    pub fn query(&mut self) -> Result<DirectoryObservation<'_>, CurrentDirectoryError> {
        use std::os::unix::ffi::OsStrExt;
        let growth = self
            .storage
            .len()
            .checked_mul(2)
            .ok_or(PathAdmissionError::Arithmetic)?;
        match nix::unistd::getcwd_into(&mut self.storage) {
            Ok(path) => Ok(DirectoryObservation::Native(Path::new(
                std::ffi::OsStr::from_bytes(path.to_bytes()),
            ))),
            Err(nix::errno::Errno::ERANGE) => Ok(DirectoryObservation::Required(growth)),
            Err(error) => Err(CurrentDirectoryError::Native(io::Error::from_raw_os_error(
                error as i32,
            ))),
        }
    }
    #[cfg(windows)]
    pub fn query(&mut self) -> Result<DirectoryObservation<'_>, CurrentDirectoryError> {
        use windows_sys::Win32::System::Environment::caller_buffer::{self, CurrentDirectory};
        match caller_buffer::current_directory_into(&mut self.storage) {
            Ok(CurrentDirectory::Written(path)) => Ok(DirectoryObservation::Wide(path)),
            Ok(CurrentDirectory::Required(capacity)) => {
                Ok(DirectoryObservation::Required(capacity as usize))
            }
            Err(error) => Err(CurrentDirectoryError::Native(io::Error::from_raw_os_error(
                error as i32,
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn admitted_current_directory_storage_can_grow_until_original_native_text_fits() {
        let expected = std::env::current_dir().unwrap();
        let mut source = CurrentDirectoryBuffer::new(1, ()).unwrap();
        loop {
            match source.query().unwrap() {
                #[cfg(unix)]
                DirectoryObservation::Native(path) => {
                    assert_eq!(path.as_os_str(), expected.as_os_str());
                    break;
                }
                #[cfg(windows)]
                DirectoryObservation::Wide(wide) => {
                    use std::os::windows::ffi::OsStrExt;
                    assert!(wide.iter().copied().eq(expected.as_os_str().encode_wide()));
                    break;
                }
                DirectoryObservation::Required(capacity) => {
                    let replacement = CurrentDirectoryBuffer::new(capacity, ()).unwrap();
                    assert!(replacement.capacity() > source.capacity());
                    source = replacement;
                }
            }
        }
    }
}
