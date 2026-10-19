use nocterm_session::{FileMetadata, local_fs::*};
use std::{
    ffi::OsStr,
    path::{Component, Path, PathBuf},
    sync::Arc,
};
#[cfg(not(unix))]
mod portable;
#[cfg(unix)]
mod unix;

/// The single private acquisition/publication capability. Browse handles cannot
/// escape to the public strict-capability contract.
#[derive(Clone)]
pub(crate) struct DirectoryCapability {
    path: PathBuf,
    #[cfg(unix)]
    handle: Arc<std::fs::File>,
    identity: DirectoryIdentity,
}
impl DirectoryCapability {
    pub(crate) fn strict(path: &Path) -> LocalResult<Self> {
        validate_path(path)?;
        #[cfg(unix)]
        {
            unix::strict(path)
        }
        #[cfg(not(unix))]
        {
            portable::strict(path)
        }
    }
    pub(crate) fn browse(path: &Path) -> LocalResult<Self> {
        #[cfg(unix)]
        {
            unix::browse(path)
        }
        #[cfg(not(unix))]
        {
            portable::browse(path)
        }
    }
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
    pub(crate) fn cursor(&self) -> LocalResult<Box<dyn LocalCursor>> {
        crate::cursor::open(self)
    }
    pub(crate) fn inspect(&self, name: &OsStr) -> LocalResult<Option<FileMetadata>> {
        check_name(name)?;
        #[cfg(unix)]
        {
            unix::metadata(self, name)
        }
        #[cfg(not(unix))]
        {
            portable::metadata(self, name)
        }
    }
    pub(crate) fn descend(&self, name: &OsStr, create: bool) -> LocalResult<Self> {
        check_name(name)?;
        #[cfg(unix)]
        {
            unix::child(self, name, create)
        }
        #[cfg(not(unix))]
        {
            portable::child(self, name, create)
        }
    }
    pub(crate) fn target_is_directory(&self, name: &OsStr) -> bool {
        #[cfg(unix)]
        {
            unix::target_is_directory(self, name)
        }
        #[cfg(not(unix))]
        {
            portable::target_is_directory(self, name)
        }
    }
    #[cfg(unix)]
    pub(crate) fn file(&self) -> &std::fs::File {
        self.handle.as_ref()
    }
}
impl LocalDirectory for DirectoryCapability {
    fn location(&self) -> PathBuf {
        self.path.clone()
    }
    fn identity(&self) -> DirectoryIdentity {
        self.identity.clone()
    }
    fn entries(&self) -> LocalResult<Box<dyn LocalCursor>> {
        self.cursor()
    }
    fn metadata(&self, name: &OsStr) -> LocalResult<Option<FileMetadata>> {
        self.inspect(name)
    }
    fn child(&self, name: &OsStr) -> LocalResult<Arc<dyn LocalDirectory>> {
        Ok(Arc::new(self.descend(name, false)?))
    }
    fn create_child(&self, name: &OsStr) -> LocalResult<Arc<dyn LocalDirectory>> {
        Ok(Arc::new(self.descend(name, true)?))
    }
    fn open_source(&self, name: &OsStr) -> LocalResult<Box<dyn LocalSource>> {
        crate::source::open_relative(self, name)
    }
    fn stage(&self) -> LocalResult<Box<dyn LocalStage>> {
        crate::stage::create(self.clone())
    }
}

pub(crate) fn check_name(name: &OsStr) -> LocalResult<()> {
    if valid_native_name(name) {
        Ok(())
    } else {
        Err(LocalFsError::InvalidName)
    }
}
pub(crate) fn validate_path(path: &Path) -> LocalResult<()> {
    if !path.is_absolute()
        || path.as_os_str().as_encoded_bytes().contains(&0)
        || path
            .components()
            .any(|part| matches!(part, Component::ParentDir))
    {
        return Err(LocalFsError::InvalidPath(path.to_owned()));
    }
    Ok(())
}
pub(crate) fn parent(path: &Path) -> LocalResult<(DirectoryCapability, &OsStr)> {
    validate_path(path)?;
    let name = path
        .file_name()
        .ok_or_else(|| LocalFsError::InvalidPath(path.to_owned()))?;
    check_name(name)?;
    let parent = path
        .parent()
        .ok_or_else(|| LocalFsError::InvalidPath(path.to_owned()))?;
    Ok((DirectoryCapability::strict(parent)?, name))
}

pub(crate) fn metadata_native(metadata: &std::fs::Metadata) -> FileMetadata {
    use nocterm_session::EntryKind;
    #[cfg(unix)]
    use std::os::unix::fs::MetadataExt;
    FileMetadata {
        kind: if metadata.is_dir() {
            EntryKind::Directory
        } else if metadata.is_file() {
            EntryKind::File
        } else {
            EntryKind::Other
        },
        is_symlink: metadata.is_symlink(),
        size: Some(metadata.len()),
        #[cfg(unix)]
        permissions: Some(metadata.mode() & 0o7777),
        #[cfg(not(unix))]
        permissions: None,
        #[cfg(unix)]
        uid: Some(metadata.uid()),
        #[cfg(not(unix))]
        uid: None,
        #[cfg(unix)]
        gid: Some(metadata.gid()),
        #[cfg(not(unix))]
        gid: None,
        modified: metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| duration.as_secs()),
    }
}

#[cfg(not(unix))]
pub(crate) use portable::is_link;
