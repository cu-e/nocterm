use crate::directory::DirectoryCapability;
use nocterm_session::local_fs::{LocalCursor, LocalFsError, LocalResult};
use ouroboros::self_referencing;
use std::{ffi::OsString, fs::File};

/// Owns an independent enumeration position and retains the captured directory's
/// complete capability chain. The iterator borrows only its own reopened handle.
#[self_referencing]
struct Cursor {
    directory: DirectoryCapability,
    file: File,
    #[borrows(mut file)]
    #[not_covariant]
    entries: fs_at::ReadDir<'this>,
}

pub(super) fn open(directory: &DirectoryCapability) -> LocalResult<Box<dyn LocalCursor>> {
    // Duplicating a handle would share its enumeration position. Reopen the
    // captured object itself so concurrent cursors never disturb each other.
    let file = crate::windows::open::enumeration(directory)?;
    let cursor = CursorTryBuilder {
        directory: directory.clone(),
        file,
        entries_builder: |file| fs_at::read_dir(file),
    }
    .try_build()
    .map_err(|error| LocalFsError::from_io(directory.path(), error))?;
    Ok(Box::new(cursor))
}

impl LocalCursor for Cursor {
    fn next(&mut self) -> LocalResult<Option<OsString>> {
        loop {
            let next = self.with_entries_mut(|entries| {
                entries
                    .next()
                    .transpose()
                    .map(|entry| entry.map(|entry| entry.name().to_os_string()))
            });
            let Some(name) =
                next.map_err(|error| LocalFsError::from_io(self.borrow_directory().path(), error))?
            else {
                return Ok(None);
            };
            // An empty directory can produce an empty first record on Windows.
            if !name.is_empty() && name != "." && name != ".." {
                return Ok(Some(name));
            }
        }
    }
}
