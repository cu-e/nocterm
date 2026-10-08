//! Single-request SFTP operations and error mapping.
use super::*;
pub(super) async fn home(sftp: &Sftp) -> Result<String, FsError> {
    sftp.raw
        .realpath(".")
        .await
        .map_err(|e| fs_error(e, "."))?
        .files
        .into_iter()
        .next()
        .map(|f| f.filename)
        .ok_or_else(|| FsError::Other("missing remote home directory".into()))
}
pub(super) async fn stat(sftp: &Sftp, path: &str) -> Result<Option<DirEntry>, FsError> {
    match sftp.raw.lstat(path).await {
        Ok(attrs) => Ok(Some(entry(path::file_name(path).into(), &attrs.attrs))),
        Err(SftpError::Status(status)) if status.status_code == StatusCode::NoSuchFile => Ok(None),
        Err(error) => Err(fs_error(error, path)),
    }
}
pub(super) fn entry(name: String, attrs: &FileAttributes) -> DirEntry {
    DirEntry {
        name,
        kind: entry_kind(attrs.file_type()),
        is_symlink: attrs.file_type() == FileType::Symlink,
        size: attrs.size,
    }
}
pub(super) fn checked_path(path: &str, mutation: bool) -> Result<&str, FsError> {
    if !path.starts_with('/') || path.contains('\0') {
        return Err(FsError::Other(
            "remote paths must be absolute without NUL".into(),
        ));
    }
    let trimmed = path.trim_end_matches('/');
    if mutation && (trimmed.is_empty() || path.split('/').any(|part| part == "." || part == "..")) {
        return Err(FsError::Other(
            "cannot modify the remote root or dot path components".into(),
        ));
    }
    Ok(if trimmed.is_empty() { "/" } else { trimmed })
}
pub(super) async fn metadata(sftp: &Sftp, path: &str) -> Result<FileMetadata, FsError> {
    let path = checked_path(path, false)?;
    let attrs = sftp
        .raw
        .lstat(path)
        .await
        .map_err(|e| fs_error(e, path))?
        .attrs;
    Ok(FileMetadata {
        kind: entry_kind(attrs.file_type()),
        is_symlink: attrs.file_type() == FileType::Symlink,
        size: attrs.size,
        permissions: attrs.permissions.map(|permissions| permissions & 0o7777),
        uid: attrs.uid,
        gid: attrs.gid,
        modified: attrs.mtime.map(u64::from),
    })
}
pub(super) async fn rename(sftp: &Sftp, old: &str, new: &str) -> Result<(), FsError> {
    let old = checked_path(old, true)?;
    let new = checked_path(new, true)?;
    // SFTP v3 RENAME refuses existing destinations; never select the replacing
    // posix-rename extension used by explicitly authorized transfer Replace.
    if stat(sftp, new).await?.is_some() {
        return Err(FsError::AlreadyExists { path: new.into() });
    }
    sftp.raw
        .rename(old, new)
        .await
        .map(|_| ())
        .map_err(|e| fs_error(e, old))
}
pub(super) async fn remove_file(sftp: &Sftp, path: &str) -> Result<(), FsError> {
    let path = checked_path(path, true)?;
    let attrs = metadata(sftp, path).await?;
    if attrs.kind == EntryKind::Directory && !attrs.is_symlink {
        return Err(FsError::Other(format!(
            "{path}: is a directory; use empty-directory removal"
        )));
    }
    sftp.raw
        .remove(path)
        .await
        .map(|_| ())
        .map_err(|e| fs_error(e, path))
}
pub(super) async fn remove_dir(sftp: &Sftp, path: &str) -> Result<(), FsError> {
    let path = checked_path(path, true)?;
    let attrs = metadata(sftp, path).await?;
    if attrs.kind != EntryKind::Directory || attrs.is_symlink {
        return Err(FsError::Other(format!("{path}: is not a real directory")));
    }
    sftp.raw
        .rmdir(path)
        .await
        .map(|_| ())
        .map_err(|e| fs_error(e, path))
}
pub(super) async fn set_permissions(
    sftp: &Sftp,
    path: &str,
    permissions: u32,
) -> Result<(), FsError> {
    let path = checked_path(path, true)?;
    if permissions & !0o7777 != 0 {
        return Err(FsError::Other(
            "permissions must contain only POSIX bits 0o0000..0o7777".into(),
        ));
    }
    let attrs = sftp
        .raw
        .lstat(path)
        .await
        .map_err(|e| fs_error(e, path))?
        .attrs;
    if attrs.permissions.is_none_or(|mode| mode & 0o170000 == 0)
        || attrs.file_type() == FileType::Symlink
    {
        return Err(FsError::Unsupported(
            "changing permissions on links or objects with unknown type".into(),
        ));
    }
    // SFTP v3 has no no-follow SETSTAT. This rejects a link observed by LSTAT,
    // but an external server-side replacement between these requests cannot be
    // excluded by this protocol. Only permissions are sent; size/owner/time stay.
    let mut updated = FileAttributes::empty();
    updated.permissions = Some(permissions);
    sftp.raw
        .setstat(path, updated)
        .await
        .map(|_| ())
        .map_err(|e| fs_error(e, path))
}

pub(super) async fn create_dir(sftp: &Sftp, path: &str) -> Result<(), FsError> {
    let path = checked_path(path, true)?;
    if let Some(entry) = stat(sftp, path).await? {
        return if entry.kind == EntryKind::Directory && !entry.is_symlink {
            Ok(())
        } else {
            Err(FsError::AlreadyExists { path: path.into() })
        };
    }
    let mut attrs = FileAttributes::empty();
    attrs.permissions = Some(0o755);
    sftp.raw
        .mkdir(path, attrs)
        .await
        .map(|_| ())
        .map_err(|e| fs_error(e, path))
}
pub(super) fn entry_kind(file_type: FileType) -> EntryKind {
    match file_type {
        FileType::Dir => EntryKind::Directory,
        FileType::File => EntryKind::File,
        _ => EntryKind::Other,
    }
}
pub(super) fn fs_error(error: SftpError, path: &str) -> FsError {
    let path = path.to_owned();
    match error {
        SftpError::Status(status) => match status.status_code {
            StatusCode::PermissionDenied => FsError::PermissionDenied { path },
            StatusCode::NoSuchFile => FsError::NotFound { path },
            _ => FsError::Other(format!("{path}: {}", status.error_message)),
        },
        other => FsError::Other(format!("{path}: {other}")),
    }
}
