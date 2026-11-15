//! Acquisition policy for captured Windows objects. Callers never translate
//! read/list intent through `read(true)`: fs_at's open_path_at ignores that flag.
use super::diagnostic;
use crate::directory::{DirectoryCapability, check_name};
use fs_at::{OpenOptions, os::windows::OpenOptionsExt as _};
use nocterm_session::local_fs::LocalResult;
use std::{ffi::OsStr, fs::File, path::Path};
use windows_sys::Wdk::Storage::FileSystem::{FILE_DIRECTORY_FILE, FILE_OPEN_REPARSE_POINT};
use windows_sys::Win32::Storage::FileSystem::{
    DELETE, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_LIST_DIRECTORY,
    FILE_READ_ATTRIBUTES, FILE_READ_DATA, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    FILE_TRAVERSE,
};

const SHARED: u32 = FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE;
const PINNED: u32 = FILE_SHARE_READ | FILE_SHARE_WRITE;
const DIRECTORY_FLAGS: u32 = FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT;

/// Acquisition needs attributes, not permission to enumerate every ancestor.
pub(crate) fn root(path: &Path) -> LocalResult<File> {
    use std::os::windows::fs::OpenOptionsExt as _;
    diagnostic::checked(
        "CreateFileW(root/open-existing)",
        path,
        FILE_READ_ATTRIBUTES,
        SHARED,
        DIRECTORY_FLAGS,
        std::fs::OpenOptions::new()
            .access_mode(FILE_READ_ATTRIBUTES)
            .share_mode(SHARED)
            .custom_flags(DIRECTORY_FLAGS)
            .open(path),
    )
}

pub(crate) fn child(parent: &DirectoryCapability, name: &OsStr, create: bool) -> LocalResult<File> {
    check_name(name)?;
    let options = FILE_DIRECTORY_FILE | FILE_OPEN_REPARSE_POINT;
    diagnostic::checked(
        if create {
            "NtCreateFile(child/open-or-create)"
        } else {
            "NtCreateFile(child/open-existing)"
        },
        &parent.path().join(name),
        FILE_READ_ATTRIBUTES,
        SHARED,
        options,
        OpenOptions::default()
            .desired_access(FILE_READ_ATTRIBUTES)
            .create_options(options)
            .create(create)
            .open_path_at(parent.file(), name),
    )
}

pub(crate) fn pin_directory(file: &File, path: &Path) -> LocalResult<File> {
    // Attribute-only handles do not participate in Windows share accounting.
    // TRAVERSE (the directory equivalent of EXECUTE) activates the no-delete
    // pin without requiring permission to list this directory's contents.
    reopen_directory(file, path, FILE_TRAVERSE | FILE_READ_ATTRIBUTES)
}

/// A distinct native handle owns its own enumeration position. Request listing
/// here only, so inaccessible ancestor listings do not prevent child traversal.
pub(crate) fn enumeration(directory: &DirectoryCapability) -> LocalResult<File> {
    reopen_directory(
        directory.file(),
        directory.path(),
        FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES,
    )
}

fn reopen_directory(file: &File, path: &Path, access: u32) -> LocalResult<File> {
    use cap_primitives::fs::{OpenOptions, OpenOptionsExt as _};
    let mut options = OpenOptions::new();
    options
        .access_mode(access)
        .share_mode(PINNED)
        .custom_flags(DIRECTORY_FLAGS);
    // Win32 ReOpenFile can reject directory handles even with BACKUP.
    // open(".") uses NT empty-name RootDirectory acquisition of this object;
    // cap_primitives::fs::reopen would call ReOpenFile and must not be used.
    diagnostic::checked(
        "NtCreateFile(reopen-directory)",
        path,
        access,
        PINNED,
        DIRECTORY_FLAGS,
        cap_primitives::fs::open(file, Path::new("."), &options),
    )
}

pub(crate) enum EntryAccess {
    Metadata,
    Source,
    Remove,
}

/// Open the selected entry itself; validation uses this owned handle, never a
/// link-following preflight. Every operation shares the same no-follow policy.
pub(crate) fn entry(
    directory: &DirectoryCapability,
    name: &OsStr,
    access: EntryAccess,
) -> LocalResult<File> {
    check_name(name)?;
    let (operation, access) = match access {
        EntryAccess::Metadata => (
            "NtCreateFile(entry-metadata/no-follow)",
            FILE_READ_ATTRIBUTES,
        ),
        EntryAccess::Source => (
            "NtCreateFile(source/no-follow)",
            FILE_READ_DATA | FILE_READ_ATTRIBUTES,
        ),
        EntryAccess::Remove => (
            "NtCreateFile(remove/no-follow)",
            DELETE | FILE_READ_ATTRIBUTES,
        ),
    };
    diagnostic::checked(
        operation,
        &directory.path().join(name),
        access,
        SHARED,
        FILE_OPEN_REPARSE_POINT,
        OpenOptions::default()
            .desired_access(access)
            .create_options(FILE_OPEN_REPARSE_POINT)
            .open_path_at(directory.file(), name),
    )
}

/// Following a link is allowed only for read-only completion hints. No directory
/// listing, ACL read or data access is necessary to determine the target kind.
pub(crate) fn target_is_directory(directory: &DirectoryCapability, name: &OsStr) -> bool {
    if check_name(name).is_err() {
        return false;
    }
    OpenOptions::default()
        .desired_access(FILE_READ_ATTRIBUTES)
        .create_options(FILE_DIRECTORY_FILE)
        .open_path_at(directory.file(), name)
        .is_ok()
}
