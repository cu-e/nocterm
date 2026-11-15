use super::*;
use crate::windows::{diagnostic, open};
use std::fs::File;
use windows_sys::Win32::Storage::FileSystem::FILE_READ_ATTRIBUTES;

fn from_file(
    path: PathBuf,
    file: File,
    ancestor: Option<Arc<DirectoryCapability>>,
) -> LocalResult<DirectoryCapability> {
    let metadata = diagnostic::checked(
        "GetFileInformationByHandle(metadata)",
        &path,
        FILE_READ_ATTRIBUTES,
        0,
        0,
        file.metadata(),
    )?;
    if super::is_link(&metadata) {
        return Err(LocalFsError::LinkRefused(path));
    }
    if !metadata.is_dir() {
        return Err(LocalFsError::NotRegular(path));
    }
    let identity = diagnostic::checked(
        "GetFileInformationByHandleEx(identity)",
        &path,
        FILE_READ_ATTRIBUTES,
        0,
        0,
        crate::windows::identity(&file),
    )?;
    Ok(DirectoryCapability {
        identity,
        path,
        handle: Arc::new(file),
        _ancestor: ancestor,
    })
}

pub(super) fn validate_publication_ancestry(directory: &DirectoryCapability) -> LocalResult<()> {
    let mut ancestor = Some(directory);
    while let Some(directory) = ancestor {
        let metadata = diagnostic::checked(
            "GetFileInformationByHandle(ancestry)",
            directory.path(),
            FILE_READ_ATTRIBUTES,
            0,
            0,
            directory.file().metadata(),
        )?;
        if super::is_link(&metadata) {
            return Err(LocalFsError::LinkRefused(directory.path().into()));
        }
        ancestor = directory._ancestor.as_deref();
    }
    Ok(())
}

pub(super) fn strict(path: &Path) -> LocalResult<DirectoryCapability> {
    let normalized: PathBuf = path.components().collect();
    let _span = tracing::info_span!(
        "windows_directory_open", pid = std::process::id(),
        requested_path = ?path, normalized_path = ?normalized,
    )
    .entered();
    if path.components().count() > 256 {
        return Err(LocalFsError::BoundExceeded("native directory ancestry"));
    }
    let mut parts = path.components();
    let Some(Component::Prefix(prefix)) = parts.next() else {
        return Err(LocalFsError::InvalidPath(path.into()));
    };
    if !matches!(parts.next(), Some(Component::RootDir)) {
        return Err(LocalFsError::InvalidPath(path.into()));
    }
    let root = PathBuf::from(prefix.as_os_str()).join(std::path::MAIN_SEPARATOR_STR);
    let file = open::root(&root)?;
    let file = open::pin_directory(&file, &root)?;
    let mut directory = from_file(root, file, None)?;
    for part in parts {
        if let Component::Normal(name) = part {
            directory = child(&directory, name, false)?;
        }
    }
    Ok(directory)
}

pub(super) fn browse(path: &Path) -> LocalResult<DirectoryCapability> {
    let canonical =
        std::fs::canonicalize(path).map_err(|error| LocalFsError::from_io(path, error))?;
    let _span = tracing::info_span!(
        "windows_directory_browse", pid = std::process::id(),
        requested_path = ?path, normalized_path = ?canonical,
    )
    .entered();
    strict(&canonical)
}

pub(super) fn child(
    parent: &DirectoryCapability,
    name: &OsStr,
    create: bool,
) -> LocalResult<DirectoryCapability> {
    check_name(name)?;
    let path = parent.path.join(name);
    if path.components().count() > 256 {
        return Err(LocalFsError::BoundExceeded("native directory ancestry"));
    }
    let opened = open::child(parent, name, create)?;
    let pinned = open::pin_directory(&opened, &path)?;
    let child = from_file(path.clone(), pinned, Some(Arc::new(parent.clone())))?;
    // Reopening with no-delete sharing may race an earlier rename. Confirm that
    // the pinned object is still the parent's selected entry before admitting it.
    let selected = open::child(parent, name, false)?;
    let selected = diagnostic::checked(
        "GetFileInformationByHandleEx(verify-identity)",
        &path,
        FILE_READ_ATTRIBUTES,
        0,
        0,
        crate::windows::identity(&selected),
    )?;
    if child.identity != selected {
        return Err(LocalFsError::LinkRefused(path));
    }
    Ok(child)
}

pub(super) fn metadata(
    parent: &DirectoryCapability,
    name: &OsStr,
) -> LocalResult<Option<FileMetadata>> {
    let path = parent.path.join(name);
    match open::entry(parent, name, open::EntryAccess::Metadata) {
        Ok(file) => {
            let metadata = diagnostic::checked(
                "GetFileInformationByHandle(entry-metadata)",
                &path,
                FILE_READ_ATTRIBUTES,
                0,
                0,
                file.metadata(),
            )?;
            let mut value = metadata_native(&metadata);
            value.is_symlink = super::is_link(&metadata);
            Ok(Some(value))
        }
        Err(LocalFsError::NotFound(_)) => Ok(None),
        Err(error) => Err(error),
    }
}

pub(super) fn target_is_directory(directory: &DirectoryCapability, name: &OsStr) -> bool {
    open::target_is_directory(directory, name)
}

pub(super) fn remove_file(parent: &DirectoryCapability, name: &OsStr) -> LocalResult<()> {
    let path = parent.path.join(name);
    let file = open::entry(parent, name, open::EntryAccess::Remove)?;
    let metadata = file
        .metadata()
        .map_err(|error| LocalFsError::from_io(&path, error))?;
    if metadata.is_dir() && !super::is_link(&metadata) {
        return Err(LocalFsError::NotRegular(path));
    }
    fs_at::os::windows::FileExt::delete_by_handle(file)
        .map_err(|(_, error)| LocalFsError::from_io(path, error))
}
