//! Presentation projection of native listings; native I/O belongs to the adapter.
pub(crate) use crate::statistics::bytes;
use nocterm_session::{EntryKind, local_fs::LocalListing};
use std::path::PathBuf;
pub(crate) const MAX_DIRECTORY_ENTRIES: usize = 100_000;
pub(crate) const MAX_DIRECTORY_NAME_BYTES: usize = 8 * 1024 * 1024;
#[derive(Clone, Debug)]
pub(crate) struct LocalEntry {
    pub name: String,
    pub path: PathBuf,
    pub kind: EntryKind,
    pub symlink: bool,
}
pub(crate) fn entries(listing: LocalListing) -> Vec<LocalEntry> {
    listing
        .entries
        .into_iter()
        .map(|entry| LocalEntry {
            name: entry.name.to_string_lossy().into_owned(),
            path: listing.directory.join(&entry.name),
            kind: entry.metadata.kind,
            symlink: entry.metadata.is_symlink,
        })
        .collect()
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::{ffi::OsString, os::unix::ffi::OsStringExt as _};
    #[test]
    fn display_name_conversion_preserves_native_operation_identity() {
        let name = OsString::from_vec(b"name-\xff".to_vec());
        let directory = PathBuf::from("/root");
        let result = entries(LocalListing {
            directory: directory.clone(),
            entries: vec![nocterm_session::local_fs::LocalDirEntry {
                name: name.clone(),
                completion_directory: false,
                metadata: nocterm_session::FileMetadata {
                    kind: EntryKind::File,
                    is_symlink: false,
                    size: Some(1),
                    permissions: None,
                    uid: None,
                    gid: None,
                    modified: None,
                },
            }],
        });
        assert_eq!(result[0].path, directory.join(name));
        assert_eq!(result[0].name, "name-�");
    }
}
