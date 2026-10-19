//! Private-directory policy facts precede allocation of their original diagnostics.
use super::{PathAdmissionError, io_error_custom_allocation_bytes};
use std::{
    fmt::{self, Write},
    io,
    path::Path,
};

#[derive(Clone, Copy, Debug, thiserror::Error)]
pub enum PrivateDirectoryReason {
    #[error("not a plain directory")]
    NotPlain,
    #[error("belongs to another user")]
    ForeignOwner,
}
impl PrivateDirectoryReason {
    pub fn kind(self) -> io::ErrorKind {
        match self {
            Self::NotPlain => io::ErrorKind::InvalidInput,
            Self::ForeignOwner => io::ErrorKind::PermissionDenied,
        }
    }
    fn suffix(self) -> &'static str {
        match self {
            Self::NotPlain => " is not a plain directory",
            Self::ForeignOwner => " belongs to another user",
        }
    }
    pub(crate) fn compatibility_error(self, path: &Path) -> io::Error {
        let message = match self {
            Self::NotPlain => format!("{} is not a plain directory", path.display()),
            Self::ForeignOwner => format!("{} belongs to another user", path.display()),
        };
        io::Error::new(self.kind(), message)
    }
}
#[derive(Debug, thiserror::Error)]
pub enum PrivateDirectoryCheckError {
    #[error(transparent)]
    Native(#[from] io::Error),
    #[error("private directory {0}")]
    Policy(PrivateDirectoryReason),
}

/// Borrowed plan includes the exact lossy-native Display output, not encoded bytes.
pub struct DirectoryMessagePlan<'a> {
    path: &'a Path,
    reason: PrivateDirectoryReason,
    capacity: usize,
}
impl<'a> DirectoryMessagePlan<'a> {
    pub fn new(path: &'a Path, reason: PrivateDirectoryReason) -> Result<Self, PathAdmissionError> {
        let mut counter = Counter {
            bytes: 0,
            overflow: false,
        };
        let result = write!(&mut counter, "{}{}", path.display(), reason.suffix());
        if counter.overflow {
            return Err(PathAdmissionError::Arithmetic);
        }
        result.map_err(PathAdmissionError::Formatting)?;
        Ok(Self {
            path,
            reason,
            capacity: counter.bytes,
        })
    }
    /// String backing plus original private StringError(String) Box and std custom.
    /// The single-field private StringError has exactly its String field layout in
    /// pinned alloc/boxed/convert.rs; fresh allocator checks bind both actual boxes.
    pub fn requested_error_bytes(&self) -> Result<usize, PathAdmissionError> {
        self.capacity
            .checked_add(std::mem::size_of::<String>())
            .and_then(|bytes| bytes.checked_add(io_error_custom_allocation_bytes()))
            .ok_or(PathAdmissionError::Arithmetic)
    }
    pub(super) fn allocate(&self) -> Result<io::Error, PathAdmissionError> {
        let mut message = String::new();
        message.try_reserve_exact(self.capacity)?;
        if message.capacity() != self.capacity {
            return Err(PathAdmissionError::CapacityProfile);
        }
        write!(
            &mut message,
            "{}{}",
            self.path.display(),
            self.reason.suffix()
        )
        .map_err(PathAdmissionError::Formatting)?;
        if message.len() != self.capacity || message.capacity() != self.capacity {
            return Err(PathAdmissionError::CapacityProfile);
        }
        Ok(io::Error::new(self.reason.kind(), message))
    }
}
struct Counter {
    bytes: usize,
    overflow: bool,
}
impl Write for Counter {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        self.bytes = self.bytes.checked_add(value.len()).ok_or_else(|| {
            self.overflow = true;
            fmt::Error
        })?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
