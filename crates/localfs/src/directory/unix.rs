use super::*;
use rustix::fs::{self, AtFlags, FileType, Mode, OFlags};
use std::os::unix::fs::MetadataExt;

fn from_file(path: PathBuf, file: std::fs::File) -> LocalResult<DirectoryCapability> {
    let metadata = file
        .metadata()
        .map_err(|error| LocalFsError::from_io(&path, error))?;
    if !metadata.is_dir() {
        return Err(LocalFsError::NotRegular(path));
    }
    Ok(DirectoryCapability {
        identity: DirectoryIdentity::native(metadata.dev(), metadata.ino()),
        path,
        handle: Arc::new(file),
    })
}
pub(super) fn strict(path: &Path) -> LocalResult<DirectoryCapability> {
    let root = fs::open(
        "/",
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|error| LocalFsError::from_io("/", error.into()))?;
    let mut held = from_file(PathBuf::from("/"), root.into())?;
    for component in path.components() {
        if let Component::Normal(name) = component {
            held = child(&held, name, false)?;
        }
    }
    Ok(held)
}
pub(super) fn browse(path: &Path) -> LocalResult<DirectoryCapability> {
    let fd = fs::open(
        path,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|error| LocalFsError::from_io(path, error.into()))?;
    from_file(path.to_owned(), fd.into())
}
pub(super) fn child(
    parent: &DirectoryCapability,
    name: &OsStr,
    create: bool,
) -> LocalResult<DirectoryCapability> {
    let path = parent.path.join(name);
    if create {
        match fs::mkdirat(parent.file(), name, Mode::from_raw_mode(0o755)) {
            Ok(()) | Err(rustix::io::Errno::EXIST) => {}
            Err(error) => return Err(LocalFsError::from_io(path, error.into())),
        }
    }
    let fd = fs::openat(
        parent.file(),
        name,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|error| classify_open(&path, error))?;
    from_file(path, fd.into())
}
fn classify_open(path: &Path, error: rustix::io::Errno) -> LocalFsError {
    if matches!(error, rustix::io::Errno::LOOP | rustix::io::Errno::NOTDIR) {
        LocalFsError::LinkRefused(path.to_owned())
    } else {
        LocalFsError::from_io(path, error.into())
    }
}
pub(super) fn metadata(
    parent: &DirectoryCapability,
    name: &OsStr,
) -> LocalResult<Option<FileMetadata>> {
    match fs::statat(parent.file(), name, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(stat) => Ok(Some(from_stat(stat))),
        Err(rustix::io::Errno::NOENT) => Ok(None),
        Err(error) => Err(LocalFsError::from_io(parent.path.join(name), error.into())),
    }
}
pub(super) fn target_is_directory(parent: &DirectoryCapability, name: &OsStr) -> bool {
    fs::statat(parent.file(), name, AtFlags::empty())
        .is_ok_and(|stat| FileType::from_raw_mode(stat.st_mode).is_dir())
}
// libc typedef widths vary across supported Unix platforms.
#[allow(clippy::unnecessary_cast)]
fn from_stat(stat: fs::Stat) -> FileMetadata {
    use nocterm_session::EntryKind;
    let kind = FileType::from_raw_mode(stat.st_mode);
    FileMetadata {
        kind: if kind.is_dir() {
            EntryKind::Directory
        } else if kind.is_file() {
            EntryKind::File
        } else {
            EntryKind::Other
        },
        is_symlink: kind == FileType::Symlink,
        size: u64::try_from(stat.st_size).ok(),
        permissions: Some(stat.st_mode as u32 & 0o7777),
        uid: Some(stat.st_uid as u32),
        gid: Some(stat.st_gid as u32),
        modified: u64::try_from(stat.st_mtime).ok(),
    }
}
