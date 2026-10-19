use super::*;
pub(crate) fn is_link(metadata: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.is_symlink()
    }
}
pub(super) fn strict(path: &Path) -> LocalResult<DirectoryCapability> {
    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component);
        if matches!(component, std::path::Component::Prefix(_)) {
            continue;
        }
        let metadata = std::fs::symlink_metadata(&current)
            .map_err(|error| LocalFsError::from_io(&current, error))?;
        if is_link(&metadata) || !metadata.is_dir() {
            return Err(LocalFsError::LinkRefused(current));
        }
    }
    Ok(DirectoryCapability {
        path: path.to_owned(),
        identity: DirectoryIdentity::portable(path.components().collect()),
    })
}
pub(super) fn browse(path: &Path) -> LocalResult<DirectoryCapability> {
    let metadata = std::fs::metadata(path).map_err(|error| LocalFsError::from_io(path, error))?;
    if !metadata.is_dir() {
        return Err(LocalFsError::NotRegular(path.to_owned()));
    }
    Ok(DirectoryCapability {
        path: path.to_owned(),
        identity: DirectoryIdentity::portable(path.to_owned()),
    })
}
pub(super) fn child(
    directory: &DirectoryCapability,
    name: &OsStr,
    create: bool,
) -> LocalResult<DirectoryCapability> {
    strict(directory.path())?;
    let path = directory.path.join(name);
    if create {
        match std::fs::create_dir(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(LocalFsError::from_io(&path, error)),
        }
    }
    strict(&path)
}
pub(super) fn metadata(
    directory: &DirectoryCapability,
    name: &OsStr,
) -> LocalResult<Option<FileMetadata>> {
    match std::fs::symlink_metadata(directory.path.join(name)) {
        Ok(metadata) => {
            let link = is_link(&metadata);
            let mut metadata = super::metadata_native(&metadata);
            metadata.is_symlink = link;
            Ok(Some(metadata))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(LocalFsError::from_io(directory.path.join(name), error)),
    }
}
pub(super) fn target_is_directory(directory: &DirectoryCapability, name: &OsStr) -> bool {
    std::fs::metadata(directory.path.join(name)).is_ok_and(|metadata| metadata.is_dir())
}
