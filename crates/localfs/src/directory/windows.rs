//! Windows held directories. A strict capability keeps an open handle to every
//! component from the volume root down. Handles are opened without
//! `FILE_SHARE_DELETE`, so while the capability lives no held component can be
//! renamed, removed or replaced by a junction; path operations below it therefore
//! resolve through the same directories that were checked. Reparse points are
//! opened as themselves and refused.
use super::*;
use std::os::windows::fs::OpenOptionsExt as _;
use windows_sys::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS,
    FILE_FLAG_OPEN_REPARSE_POINT, FILE_LIST_DIRECTORY, FILE_READ_ATTRIBUTES, FILE_SHARE_READ,
    FILE_SHARE_WRITE, SYNCHRONIZE,
};

/// One held directory and, through `parent`, every ancestor above it.
pub(crate) struct Held {
    _file: std::fs::File,
    _parent: Option<Arc<Held>>,
}

/// Open `path` as a directory without following a final reparse point. The
/// requested list access takes part in share accounting, so the omitted delete
/// sharing actually pins the directory's name.
fn open(path: &Path, parent: Option<Arc<Held>>) -> LocalResult<(Arc<Held>, DirectoryIdentity)> {
    let file = std::fs::OpenOptions::new()
        .access_mode(FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES | SYNCHRONIZE)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .map_err(|error| LocalFsError::from_io(path, error))?;
    // The opened handle, not the path, supplies the attributes and identity.
    let information = winapi_util::file::information(&file)
        .map_err(|error| LocalFsError::from_io(path, error))?;
    let attributes = information.file_attributes();
    if attributes & u64::from(FILE_ATTRIBUTE_REPARSE_POINT) != 0
        || attributes & u64::from(FILE_ATTRIBUTE_DIRECTORY) == 0
    {
        return Err(LocalFsError::LinkRefused(path.to_owned()));
    }
    Ok((
        Arc::new(Held {
            _file: file,
            _parent: parent,
        }),
        DirectoryIdentity::native(information.volume_serial_number(), information.file_index()),
    ))
}

pub(super) fn strict(path: &Path) -> LocalResult<DirectoryCapability> {
    let mut current = PathBuf::new();
    let mut held = None;
    for component in path.components() {
        current.push(component);
        if matches!(component, Component::Prefix(_)) {
            continue;
        }
        let parent = held.take().map(|(handle, _)| handle);
        held = Some(open(&current, parent)?);
    }
    let (handle, identity) = held.ok_or_else(|| LocalFsError::InvalidPath(path.to_owned()))?;
    Ok(DirectoryCapability {
        path: path.to_owned(),
        held: Some(handle),
        identity,
    })
}

pub(super) fn child(
    directory: &DirectoryCapability,
    name: &OsStr,
    create: bool,
) -> LocalResult<DirectoryCapability> {
    // A browse capability followed links and holds nothing; re-establish the chain.
    let parent = match &directory.held {
        Some(held) => held.clone(),
        None => strict(directory.path())?
            .held
            .ok_or_else(|| LocalFsError::InvalidPath(directory.path().to_owned()))?,
    };
    let path = directory.path.join(name);
    if create {
        match std::fs::create_dir(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(LocalFsError::from_io(&path, error)),
        }
    }
    let (handle, identity) = open(&path, Some(parent))?;
    Ok(DirectoryCapability {
        path,
        held: Some(handle),
        identity,
    })
}

pub(super) fn remove_file(directory: &DirectoryCapability, name: &OsStr) -> LocalResult<()> {
    let _pinned = match directory.held {
        Some(_) => None,
        None => Some(strict(directory.path())?),
    };
    let path = directory.path.join(name);
    if super::portable::metadata(directory, name)?.is_some_and(|metadata| {
        metadata.kind == nocterm_session::EntryKind::Directory && !metadata.is_symlink
    }) {
        return Err(LocalFsError::NotRegular(path));
    }
    std::fs::remove_file(&path).map_err(|error| LocalFsError::from_io(path, error))
}
