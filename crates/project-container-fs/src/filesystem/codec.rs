use nocterm_session::{DirEntry, EntryKind, FileMetadata, FsError};
pub(super) fn text(bytes: &[u8]) -> Result<&str, FsError> {
    std::str::from_utf8(bytes).map_err(|_| {
        FsError::Other(
            "container returned an unrepresentable filename or malformed metadata".into(),
        )
    })
}
pub(super) fn metadata(bytes: &[u8]) -> Result<FileMetadata, FsError> {
    let data = text(bytes)?.trim();
    let fields: Vec<_> = data.split(':').collect();
    if fields.len() != 8 {
        return Err(malformed());
    }
    let mode = u32::from_str_radix(fields[2], 16).map_err(|_| malformed())?;
    Ok(FileMetadata {
        kind: match mode & 0o170000 {
            0o040000 => EntryKind::Directory,
            0o100000 => EntryKind::File,
            _ => EntryKind::Other,
        },
        is_symlink: mode & 0o170000 == 0o120000,
        size: Some(fields[3].parse().map_err(|_| malformed())?),
        permissions: Some(u32::from_str_radix(fields[4], 8).map_err(|_| malformed())?),
        uid: Some(fields[5].parse().map_err(|_| malformed())?),
        gid: Some(fields[6].parse().map_err(|_| malformed())?),
        modified: fields[7].parse::<u64>().ok(),
    })
}
pub(super) fn entry(name: String, metadata: FileMetadata) -> DirEntry {
    DirEntry {
        name,
        kind: metadata.kind,
        is_symlink: metadata.is_symlink,
        size: metadata.size,
    }
}
pub(super) fn listing(bytes: &[u8]) -> Result<Vec<DirEntry>, FsError> {
    if !bytes.is_empty() && bytes.last() != Some(&0) {
        return Err(malformed());
    }
    let parts: Vec<_> = bytes
        .strip_suffix(&[0])
        .unwrap_or(bytes)
        .split(|byte| *byte == 0)
        .filter(|_| !bytes.is_empty())
        .collect();
    let (pairs, remainder) = parts.as_chunks::<2>();
    if !remainder.is_empty() || pairs.len() > 100_000 {
        return Err(malformed());
    }
    let mut names = 0usize;
    let mut entries = Vec::with_capacity(pairs.len());
    for [name, details] in pairs {
        names = names.saturating_add(name.len());
        if name.is_empty() || name.len() > 4096 || name.contains(&b'/') || names > 8 * 1024 * 1024 {
            return Err(malformed());
        }
        entries.push(entry(text(name)?.into(), metadata(details)?));
    }
    Ok(entries)
}
pub(super) fn identity(bytes: &[u8]) -> Result<String, FsError> {
    let value = text(bytes)?.trim();
    let parts: Vec<_> = value.split(':').collect();
    if parts.len() != 2
        || parts
            .iter()
            .any(|part| part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return Err(malformed());
    }
    Ok(value.into())
}
fn malformed() -> FsError {
    FsError::Other("container returned malformed or excessive filesystem data".into())
}
