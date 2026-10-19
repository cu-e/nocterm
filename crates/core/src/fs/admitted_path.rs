//! Exact requested path allocations retain their caller's original admission credit.
use std::{
    collections::TryReserveError,
    ffi::OsString,
    path::{Path, PathBuf},
};
mod join;
mod leaf;
pub use join::PreparedPathJoin;
mod wide;
pub use leaf::LeafPathPlan;
pub use wide::WidePathPlan;

#[derive(Debug, thiserror::Error)]
pub enum PathAdmissionError {
    #[error("native path diagnostic formatting failed: {0}")]
    Formatting(#[from] std::fmt::Error),
    #[error("native path allocation size overflow")]
    Arithmetic,
    #[error("native path allocation failed: {0}")]
    Allocation(#[from] TryReserveError),
    #[error("native path capacity differs from the pinned allocation profile")]
    CapacityProfile,
    #[error("native leaf must be one ordinary path component")]
    InvalidLeaf,
    #[error("native path plan does not describe the supplied source")]
    SourceMismatch,
}

/// The path physically drops before its inseparable supplied lifetime credit.
/// No raw owned path can escape independently of that credit.
pub struct AdmittedPath<Credit> {
    path: PathBuf,
    credit: Credit,
}
impl<Credit> AdmittedPath<Credit> {
    /// Requested bytes to copy this native spelling without normalization.
    pub fn copy_capacity(path: &Path) -> usize {
        path.as_os_str().as_encoded_bytes().len()
    }
    /// Caller admits `copy_capacity(path)` bytes before calling this constructor.
    pub fn try_copy(path: &Path, credit: Credit) -> Result<Self, PathAdmissionError> {
        let capacity = Self::copy_capacity(path);
        let mut encoded = OsString::new();
        encoded.try_reserve_exact(capacity)?;
        if encoded.capacity() != capacity {
            return Err(PathAdmissionError::CapacityProfile);
        }
        encoded.push(path.as_os_str());
        Ok(Self {
            path: encoded.into(),
            credit,
        })
    }
    pub fn as_path(&self) -> &Path {
        &self.path
    }
    pub fn capacity(&self) -> usize {
        self.path.capacity()
    }
    pub(super) fn into_parts(self) -> (PathBuf, Credit) {
        (self.path, self.credit)
    }
}
impl<Credit> AsRef<Path> for AdmittedPath<Credit> {
    fn as_ref(&self) -> &Path {
        self.as_path()
    }
}

impl<Credit> AdmittedPath<Credit> {
    /// Exact requested native capacity for appending one ordinary leaf.
    pub fn leaf_capacity(base: &Path, leaf: &std::ffi::OsStr) -> Result<usize, PathAdmissionError> {
        Ok(LeafPathPlan::new(base, leaf)?.capacity())
    }
    /// The caller prepays leaf_capacity before allocation and native path assembly.
    pub fn try_join_leaf(
        base: &Path,
        leaf: &std::ffi::OsStr,
        credit: Credit,
    ) -> Result<Self, PathAdmissionError> {
        LeafPathPlan::new(base, leaf)?.allocate(credit)
    }
}

#[cfg(test)]
mod tests;
