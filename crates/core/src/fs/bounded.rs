//! Reads one regular file without following a final link, within a size bound.

use std::{
    fs::{File, Metadata, OpenOptions},
    io::{self, Read as _},
    path::{Path, PathBuf},
};

/// A bounded read refused the path or failed.
#[derive(Debug, thiserror::Error)]
pub enum BoundedReadError {
    /// The path is not a regular file, or its final component is a link.
    #[error("{}: not a regular file", path.display())]
    NotRegular { path: PathBuf },
    /// The file holds more than the allowed bytes.
    #[error("{}: larger than {cap} bytes", path.display())]
    TooLarge { path: PathBuf, cap: usize },
    #[error("{}: {source}", path.display())]
    Io { path: PathBuf, source: io::Error },
}

impl BoundedReadError {
    fn io(path: &Path, source: io::Error) -> Self {
        Self::Io {
            path: path.to_owned(),
            source,
        }
    }

    /// The operating-system error kind, when the failure came from the system.
    pub fn io_kind(&self) -> Option<io::ErrorKind> {
        match self {
            Self::Io { source, .. } => Some(source.kind()),
            Self::NotRegular { .. } | Self::TooLarge { .. } => None,
        }
    }
}

/// Reads a regular file of at most `cap` bytes, never following a link at the final component.
///
/// The file is opened first (`O_NOFOLLOW | O_NONBLOCK | O_CLOEXEC` on Unix, with
/// `FILE_FLAG_OPEN_REPARSE_POINT` and a reparse-point check on Windows) and its kind and size
/// are read from the open handle, so a path swapped after the check cannot change what is
/// read. A named pipe or device is refused without blocking. The read itself stops one byte
/// past `cap`, so a file that grows while it is read cannot exceed the bound.
///
/// Parent directories must be trusted: this is not a pinned-directory traversal API. Callers
/// that hash the content hash the returned buffer, which is the exact content they parse.
pub fn read_regular_bounded(path: &Path, cap: usize) -> Result<Vec<u8>, BoundedReadError> {
    let file = open_without_following(path).map_err(|source| {
        if is_link_refusal(&source) {
            BoundedReadError::NotRegular {
                path: path.to_owned(),
            }
        } else {
            BoundedReadError::io(path, source)
        }
    })?;
    let metadata = file
        .metadata()
        .map_err(|source| BoundedReadError::io(path, source))?;
    if !metadata.is_file() || is_reparse_point(&metadata) {
        return Err(BoundedReadError::NotRegular {
            path: path.to_owned(),
        });
    }
    let limit = u64::try_from(cap).unwrap_or(u64::MAX);
    if metadata.len() > limit {
        return Err(BoundedReadError::TooLarge {
            path: path.to_owned(),
            cap,
        });
    }
    let mut bytes = Vec::with_capacity(usize::try_from(metadata.len()).unwrap_or(0));
    file.take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|source| BoundedReadError::io(path, source))?;
    if bytes.len() > cap {
        return Err(BoundedReadError::TooLarge {
            path: path.to_owned(),
            cap,
        });
    }
    Ok(bytes)
}

#[cfg(unix)]
fn open_without_following(path: &Path) -> io::Result<File> {
    use nix::fcntl::OFlag;
    use std::os::unix::fs::OpenOptionsExt as _;
    OpenOptions::new()
        .read(true)
        .custom_flags(
            (OFlag::O_NOFOLLOW | OFlag::O_NONBLOCK | OFlag::O_NOCTTY | OFlag::O_CLOEXEC).bits(),
        )
        .open(path)
}

#[cfg(windows)]
fn open_without_following(path: &Path) -> io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt as _;
    /// `FILE_FLAG_OPEN_REPARSE_POINT`: open the link itself, not its target.
    const OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    OpenOptions::new()
        .read(true)
        .custom_flags(OPEN_REPARSE_POINT)
        .open(path)
}

#[cfg(not(any(unix, windows)))]
fn open_without_following(path: &Path) -> io::Result<File> {
    OpenOptions::new().read(true).open(path)
}

/// `O_NOFOLLOW` reports a final link as ELOOP (Linux) or EMLINK (BSD, macOS).
#[cfg(unix)]
fn is_link_refusal(error: &io::Error) -> bool {
    use nix::errno::Errno;
    error
        .raw_os_error()
        .is_some_and(|code| matches!(Errno::from_raw(code), Errno::ELOOP | Errno::EMLINK))
}

#[cfg(not(unix))]
fn is_link_refusal(_: &io::Error) -> bool {
    false
}

#[cfg(windows)]
fn is_reparse_point(metadata: &Metadata) -> bool {
    use std::os::windows::fs::MetadataExt as _;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_reparse_point(_: &Metadata) -> bool {
    false
}
