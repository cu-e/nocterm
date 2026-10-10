use crate::directory::{check_name, parent};
use nocterm_session::{FileMetadata, local_fs::*};
#[cfg(unix)]
use rustix::fs::{self, AtFlags, FileType};
use std::{ffi::OsStr, path::Path};

pub(crate) fn metadata(path: &Path) -> LocalResult<FileMetadata> {
    let (directory, name) = parent(path)?;
    directory
        .inspect(name)?
        .ok_or_else(|| LocalFsError::NotFound(path.to_owned()))
}
pub(crate) fn rename(path: &Path, name: &OsStr) -> LocalResult<()> {
    check_name(name)?;
    let (directory, old) = parent(path)?;
    #[cfg(any(
        target_os = "linux",
        target_os = "android",
        target_os = "macos",
        target_os = "ios"
    ))]
    return fs::renameat_with(
        directory.file(),
        old,
        directory.file(),
        name,
        fs::RenameFlags::NOREPLACE,
    )
    .map_err(|error| LocalFsError::from_io(path, error.into()));
    #[cfg(windows)]
    {
        let _ = old;
        return atomicwrites::move_atomic(path, &directory.path().join(name))
            .map_err(|error| LocalFsError::from_io(path, error));
    }
    #[cfg(not(any(
        target_os = "linux",
        target_os = "android",
        target_os = "macos",
        target_os = "ios",
        windows
    )))]
    {
        let _ = (directory, old);
        Err(LocalFsError::Unsupported(
            "atomic rename without replacement",
        ))
    }
}
pub(crate) fn permissions(path: &Path, mode: u32) -> LocalResult<()> {
    if mode & !0o7777 != 0 {
        return Err(LocalFsError::Unsupported("invalid POSIX permission mask"));
    }
    let (directory, name) = parent(path)?;
    if directory
        .inspect(name)?
        .is_some_and(|metadata| metadata.is_symlink)
    {
        return Err(LocalFsError::LinkRefused(path.to_owned()));
    }
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        let mode = nix::libc::mode_t::try_from(mode)
            .map_err(|_| LocalFsError::Unsupported("invalid POSIX permission mask"))?;
        nix::sys::stat::fchmodat(
            Some(directory.file().as_raw_fd()),
            name,
            nix::sys::stat::Mode::from_bits_truncate(mode),
            nix::sys::stat::FchmodatFlags::NoFollowSymlink,
        )
        .map_err(|error| LocalFsError::from_io(path, error.into()))
    }
    #[cfg(not(unix))]
    {
        let _ = (directory, name);
        Err(LocalFsError::Unsupported("POSIX permissions"))
    }
}

struct Removal {
    cancel: CancelFlag,
    limits: RemovalLimits,
    visited: usize,
    removed: usize,
}
impl Removal {
    fn enter(&mut self, depth: usize) -> LocalResult<()> {
        self.cancel.check()?;
        self.visited = self.visited.saturating_add(1);
        if depth > self.limits.depth() || self.visited > self.limits.entries() {
            return Err(LocalFsError::BoundExceeded("deletion"));
        }
        Ok(())
    }
    #[cfg(unix)]
    fn entry(
        &mut self,
        directory: &crate::directory::DirectoryCapability,
        name: &OsStr,
        depth: usize,
    ) -> LocalResult<()> {
        self.enter(depth)?;
        let path = directory.path().join(name);
        let stat = fs::statat(directory.file(), name, AtFlags::SYMLINK_NOFOLLOW)
            .map_err(|error| LocalFsError::from_io(&path, error.into()))?;
        let is_directory = FileType::from_raw_mode(stat.st_mode).is_dir();
        if is_directory {
            let child = directory.descend(name, false)?;
            let mut cursor = child.cursor()?;
            while let Some(name) = cursor.next()? {
                self.entry(&child, &name, depth + 1)?;
            }
        }
        self.cancel.check()?;
        fs::unlinkat(
            directory.file(),
            name,
            if is_directory {
                AtFlags::REMOVEDIR
            } else {
                AtFlags::empty()
            },
        )
        .map_err(|error| LocalFsError::from_io(path, error.into()))?;
        self.removed = self.removed.saturating_add(1);
        Ok(())
    }
    #[cfg(not(unix))]
    fn entry(
        &mut self,
        directory: &crate::directory::DirectoryCapability,
        name: &OsStr,
        depth: usize,
    ) -> LocalResult<()> {
        self.enter(depth)?;
        let path = directory.path().join(name);
        let metadata = std::fs::symlink_metadata(&path)
            .map_err(|error| LocalFsError::from_io(&path, error))?;
        let is_directory = metadata.is_dir() && !crate::directory::is_link(&metadata);
        if is_directory {
            let child = directory.descend(name, false)?;
            let mut cursor = child.cursor()?;
            while let Some(name) = cursor.next()? {
                self.entry(&child, &name, depth + 1)?;
            }
        }
        self.cancel.check()?;
        crate::directory::DirectoryCapability::strict(directory.path())?;
        let current = std::fs::symlink_metadata(&path)
            .map_err(|error| LocalFsError::from_io(&path, error))?;
        if is_directory && (!current.is_dir() || crate::directory::is_link(&current)) {
            return Err(LocalFsError::LinkRefused(path));
        }
        #[cfg(windows)]
        let remove_dir = {
            use std::os::windows::fs::MetadataExt;
            current.file_attributes() & 0x10 != 0
        };
        #[cfg(not(windows))]
        let remove_dir = is_directory;
        let result = if remove_dir {
            std::fs::remove_dir(&path)
        } else {
            std::fs::remove_file(&path)
        };
        result.map_err(|error| LocalFsError::from_io(path, error))?;
        self.removed = self.removed.saturating_add(1);
        Ok(())
    }
}
pub(crate) fn remove(
    path: &Path,
    limits: RemovalLimits,
    cancel: &CancelFlag,
) -> LocalResult<RemovalOutcome> {
    cancel.check()?;
    let (directory, name) = parent(path)?;
    let mut removal = Removal {
        cancel: cancel.clone(),
        limits,
        visited: 0,
        removed: 0,
    };
    match removal.entry(&directory, name, 0) {
        Ok(()) => Ok(RemovalOutcome {
            removed: removal.removed,
        }),
        Err(source) if removal.removed == 0 => Err(source),
        Err(source) => Err(LocalFsError::PartialRemoval {
            removed: removal.removed,
            source: Box::new(source),
        }),
    }
}
