use crate::directory::DirectoryCapability;
use nocterm_session::local_fs::*;
use std::{ffi::OsString, path::PathBuf};
#[cfg(unix)]
struct Cursor {
    entries: rustix::fs::Dir,
    path: PathBuf,
}
#[cfg(not(unix))]
struct Cursor {
    entries: std::fs::ReadDir,
    path: PathBuf,
}

pub(crate) fn open(directory: &DirectoryCapability) -> LocalResult<Box<dyn LocalCursor>> {
    #[cfg(unix)]
    let entries = rustix::fs::Dir::read_from(directory.file())
        .map_err(|error| LocalFsError::from_io(directory.path(), error.into()))?;
    #[cfg(not(unix))]
    let entries = std::fs::read_dir(directory.path())
        .map_err(|error| LocalFsError::from_io(directory.path(), error))?;
    Ok(Box::new(Cursor {
        entries,
        path: directory.path().to_owned(),
    }))
}
impl LocalCursor for Cursor {
    fn next(&mut self) -> LocalResult<Option<OsString>> {
        loop {
            let Some(entry) = self.entries.next() else {
                return Ok(None);
            };
            #[cfg(unix)]
            let name = {
                use std::os::unix::ffi::OsStrExt;
                OsString::from(std::ffi::OsStr::from_bytes(
                    entry
                        .map_err(|error| LocalFsError::from_io(&self.path, error.into()))?
                        .file_name()
                        .to_bytes(),
                ))
            };
            #[cfg(not(unix))]
            let name = entry
                .map_err(|error| LocalFsError::from_io(&self.path, error))?
                .file_name();
            if name != "." && name != ".." {
                return Ok(Some(name));
            }
        }
    }
}
