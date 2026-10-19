//! Nocterm addition: per-instance owners for the original anonymous engine.
use std::{
    ffi::OsStr,
    io,
    path::{Path, PathBuf},
};

/// A native adapter supplies genuinely admitted paths and conversion lifetimes.
/// No native state or global account is discovered by the tempfile engine.
pub trait AnonymousOperations {
    type Path: AsRef<Path>;
    type Credit;
    type Error;
    fn current_dir(&mut self) -> Result<Self::Path, Self::Error>;
    fn join(&mut self, base: &Path, right: &Path) -> Result<Self::Path, Self::Error>;
    fn name_buffer(&mut self, capacity: usize) -> Result<Name<Self::Credit>, Self::Error>;
    fn native_path(&mut self, path: &Path) -> Result<Self::Credit, Self::Error>;
    fn native_error(&mut self, error: io::Error) -> Self::Error;
    fn error_kind(error: &Self::Error) -> Option<io::ErrorKind>;
    fn exhausted(&mut self, base: &Path) -> Self::Error;
}

pub(crate) struct Legacy;
impl AnonymousOperations for Legacy {
    type Path = PathBuf;
    type Credit = ();
    type Error = io::Error;
    fn current_dir(&mut self) -> io::Result<PathBuf> {
        std::env::current_dir()
    }
    fn join(&mut self, base: &Path, right: &Path) -> io::Result<PathBuf> {
        Ok(base.join(right))
    }
    fn name_buffer(&mut self, capacity: usize) -> io::Result<Name<()>> {
        Ok(Name {
            value: std::ffi::OsString::with_capacity(capacity),
            _credit: (),
        })
    }
    fn native_path(&mut self, _: &Path) -> io::Result<()> {
        Ok(())
    }
    fn native_error(&mut self, error: io::Error) -> io::Error {
        error
    }
    fn error_kind(error: &io::Error) -> Option<io::ErrorKind> {
        Some(error.kind())
    }
    fn exhausted(&mut self, base: &Path) -> io::Error {
        exhausted_with_owned_path(base.to_owned())
    }
}

/// Original retry-exhaustion error and original private PathError construction.
/// Controlled callers prepay the supplied path and both original custom bodies.
pub fn exhausted_with_owned_path(path: PathBuf) -> io::Error {
    crate::error::error_with_owned_path(
        io::Error::new(
            io::ErrorKind::AlreadyExists,
            "too many temporary files exist",
        ),
        path,
    )
}

/// Run the same platform anonymous-file engine with explicit per-instance owners.
/// The supplied base stays borrowed; callers own its actual allocation lifetime.
#[cfg(unix)]
pub fn tempfile_in<O: AnonymousOperations>(
    dir: &Path,
    operations: &mut O,
) -> Result<std::fs::File, O::Error> {
    crate::file::imp::create_with_operations(dir, operations)
}

pub struct Name<C> {
    value: std::ffi::OsString,
    _credit: C,
}
impl<C> Name<C> {
    /// The caller admits capacity before creating this original name backing.
    pub fn try_with_capacity(capacity: usize, credit: C) -> Result<Self, NameAllocationError> {
        let mut value = std::ffi::OsString::new();
        value
            .try_reserve_exact(capacity)
            .map_err(NameAllocationError::Allocation)?;
        if value.capacity() != capacity {
            return Err(NameAllocationError::CapacityProfile);
        }
        Ok(Self {
            value,
            _credit: credit,
        })
    }
    pub(crate) fn push(&mut self, value: &OsStr) {
        self.value.push(value);
    }
    pub(crate) fn as_os_str(&self) -> &OsStr {
        &self.value
    }
}
#[derive(Debug)]
pub enum NameAllocationError {
    Allocation(std::collections::TryReserveError),
    CapacityProfile,
}
impl std::fmt::Display for NameAllocationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Allocation(error) => write!(f, "temporary name allocation failed: {error}"),
            Self::CapacityProfile => {
                f.write_str("temporary name capacity differs from its admitted profile")
            }
        }
    }
}
impl std::error::Error for NameAllocationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Allocation(error) => Some(error),
            Self::CapacityProfile => None,
        }
    }
}
