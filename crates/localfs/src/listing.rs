use crate::directory::DirectoryCapability;
use nocterm_session::{EntryKind, local_fs::*};
use std::path::Path;

pub(crate) fn browse(
    path: &Path,
    limits: ListingLimits,
    cancel: &CancelFlag,
) -> LocalResult<LocalListing> {
    cancel.check()?;
    let directory = DirectoryCapability::browse(path)?;
    let mut cursor = directory.cursor()?;
    let mut entries = Vec::new();
    let mut name_bytes = 0usize;
    while let Some(name) = cursor.next()? {
        cancel.check()?;
        let metadata = directory
            .inspect(&name)?
            .ok_or_else(|| LocalFsError::NotFound(path.join(&name)))?;
        // Display-name memory is the same Explorer bound; native names stay exact.
        name_bytes = name_bytes.saturating_add(name.to_string_lossy().len());
        if entries.len() >= limits.entries() || name_bytes > limits.name_bytes() {
            return Err(LocalFsError::BoundExceeded("Explorer listing"));
        }
        let completion_directory = metadata.kind == EntryKind::Directory
            || metadata.is_symlink && directory.target_is_directory(&name);
        entries.push(LocalDirEntry {
            name,
            metadata,
            completion_directory,
        });
    }
    entries.sort_by_cached_key(|entry| {
        (
            entry.metadata.kind != EntryKind::Directory,
            entry.name.to_string_lossy().to_lowercase(),
            entry.name.clone(),
        )
    });
    Ok(LocalListing {
        directory: path.to_owned(),
        entries,
    })
}
