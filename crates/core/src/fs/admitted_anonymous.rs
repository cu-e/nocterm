//! Original anonymous-file exhaustion errors retain their exact paid backings.
use super::{AdmittedPath, PathAdmissionError, io_error_custom_allocation_bytes};
use std::{error::Error, fmt, io};

pub struct AdmittedAnonymousError<Credit> {
    original: io::Error,
    _path: Credit,
    _nested: Credit,
}
impl<Credit> AdmittedAnonymousError<Credit> {
    /// Original String bytes + boxed StringError + its Custom + PathError + outer Custom.
    /// The diagnostic path's actual capacity is a separate supplied liability.
    pub fn nested_allocation_bytes() -> Result<usize, PathAdmissionError> {
        std::mem::size_of::<String>()
            .checked_add("too many temporary files exist".len())
            .and_then(|bytes| bytes.checked_add(io_error_custom_allocation_bytes()))
            .and_then(|bytes| bytes.checked_add(tempfile::path_error_allocation_bytes()))
            .and_then(|bytes| bytes.checked_add(io_error_custom_allocation_bytes()))
            .ok_or(PathAdmissionError::Arithmetic)
    }
    /// Both credits are admitted before the unchanged stock constructors.
    pub fn exhaustion(path: AdmittedPath<Credit>, nested: Credit) -> Self {
        let (path, path_credit) = path.into_parts();
        Self {
            original: tempfile::anonymous::exhausted_with_owned_path(path),
            _path: path_credit,
            _nested: nested,
        }
    }
    pub fn native_source(&self) -> &io::Error {
        &self.original
    }
}
impl<Credit> fmt::Debug for AdmittedAnonymousError<Credit> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.original, f)
    }
}
impl<Credit> fmt::Display for AdmittedAnonymousError<Credit> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.original, f)
    }
}
impl<Credit> Error for AdmittedAnonymousError<Credit> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.original)
    }
}
