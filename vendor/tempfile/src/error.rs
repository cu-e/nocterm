use std::path::PathBuf;
use std::{error, fmt, io};

#[derive(Debug)]
struct PathError {
    path: PathBuf,
    err: io::Error,
}

impl fmt::Display for PathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} at path {:?}", self.err, self.path)
    }
}

impl error::Error for PathError {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        self.err.source()
    }
}

pub(crate) trait IoResultExt<T> {
    fn with_err_path<F, P>(self, path: F) -> Self
    where
        F: FnOnce() -> P,
        P: Into<PathBuf>;
}

impl<T> IoResultExt<T> for Result<T, io::Error> {
    fn with_err_path<F, P>(self, path: F) -> Self
    where
        F: FnOnce() -> P,
        P: Into<PathBuf>,
    {
        self.map_err(|e| error_with_owned_path(e, path().into()))
    }
}

/// Requested bytes of the original private PathError allocation.
/// The owned path and the standard library io::Error custom allocation are
/// separate liabilities; callers must prepay all three before wrapping.
pub fn path_error_allocation_bytes() -> usize {
    std::mem::size_of::<PathError>()
}

/// Wrap a native error with its already owned operation path.
///
/// Uses the original private PathError type and its exact Display/source policy.
/// Moving the path performs no clone; io::Error::new allocates its two error
/// bodies, which controlled callers must admit before invoking this function.
pub fn error_with_owned_path(error: io::Error, path: PathBuf) -> io::Error {
    io::Error::new(error.kind(), PathError { path, err: error })
}
