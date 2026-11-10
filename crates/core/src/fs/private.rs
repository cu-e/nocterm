//! Validate the actual opened descriptor before a private owner reads secrets.
use super::bounded::{BoundedReadError, open_regular};
use std::{
    fs::{File, Metadata},
    io,
    path::Path,
};

#[derive(Debug, thiserror::Error)]
pub enum PrivateFileError {
    #[error("the private file could not be opened")]
    Open(#[source] BoundedReadError),
    #[error("the opened private file could not be inspected")]
    Inspect(#[source] io::Error),
    #[error("the opened file is not an owner-only single-link regular file")]
    Policy,
    #[error("private file ownership checks are unavailable on this platform")]
    Unsupported,
}
impl PrivateFileError {
    pub fn io_kind(&self) -> Option<io::ErrorKind> {
        match self {
            Self::Open(error) => error.io_kind(),
            Self::Inspect(error) => Some(error.kind()),
            Self::Policy | Self::Unsupported => None,
        }
    }
}

/// Opens without following the final link and checks the opened inode's policy.
/// On Unix only current effective-UID ownership, one link and exact0400/0600
/// permissions are accepted. Other platforms refuse before opening the path.
///
/// No secret bytes are read here. The owner supplies its fixed zeroizing buffer
/// and bounds reads, including one-byte growth/overflow detection. Parents must
/// be trusted; this does not pin directory ancestry or create process isolation.
pub fn open_private_regular(path: &Path) -> Result<File, PrivateFileError> {
    if !cfg!(unix) {
        return Err(PrivateFileError::Unsupported);
    }
    let (file, metadata) = open_regular(path).map_err(PrivateFileError::Open)?;
    check(&metadata)?;
    Ok(file)
}

/// Applies the same policy to a held descriptor, including after publication.
/// A refusal after publication does not remove or roll back that published file.
pub fn require_private_regular(file: &File) -> Result<(), PrivateFileError> {
    if !cfg!(unix) {
        return Err(PrivateFileError::Unsupported);
    }
    let metadata = file.metadata().map_err(PrivateFileError::Inspect)?;
    check(&metadata)
}

#[cfg(unix)]
fn check(metadata: &Metadata) -> Result<(), PrivateFileError> {
    use std::os::unix::fs::MetadataExt as _;
    if metadata.is_file()
        && metadata.uid() == nix::unistd::Uid::effective().as_raw()
        && metadata.nlink() == 1
        && matches!(metadata.mode() & 0o7777, 0o400 | 0o600)
    {
        Ok(())
    } else {
        Err(PrivateFileError::Policy)
    }
}
#[cfg(not(unix))]
fn check(_: &Metadata) -> Result<(), PrivateFileError> {
    Err(PrivateFileError::Unsupported)
}
