//! Safe native handle helpers and a bounded publication path guard.
use crate::directory::DirectoryCapability;
use fs_at::{
    OpenOptions,
    os::windows::{FileExt, OpenOptionsExt},
};
use nocterm_session::local_fs::{LocalFsError, LocalResult};
use std::{fs::File, io, os::windows::io::AsHandle};
use windows_sys::Win32::Storage::FileSystem::{FILE_READ_ATTRIBUTES, FILE_WRITE_DATA};
use winx::file::{AccessMode, Flags, ShareMode, reopen_file};

pub(crate) mod diagnostic;
pub(crate) mod open;

pub(crate) fn identity(file: &File) -> io::Result<nocterm_session::local_fs::DirectoryIdentity> {
    use nocterm_session::local_fs::DirectoryIdentity;
    Ok(match prikk_ffi::identity_of(file)? {
        prikk_ffi::FileIdentity::Id128 {
            volume_serial_number,
            file_id,
        } => DirectoryIdentity::native_wide(volume_serial_number, u128::from_le_bytes(file_id)),
        prikk_ffi::FileIdentity::Id64 {
            volume_serial_number,
            file_index,
        } => DirectoryIdentity::native(u64::from(volume_serial_number), file_index),
    })
}

/// Ancestor handles deny deletion; this locked child also keeps the leaf nonempty,
/// so it cannot become a junction while path-based atomic rename is in progress.
/// All guard creation and deletion are relative/handle-based.
pub(crate) struct PublicationGuard {
    file: Option<File>,
}
impl PublicationGuard {
    pub(crate) fn new(directory: &DirectoryCapability) -> LocalResult<Self> {
        let mut bytes = [0; 16];
        getrandom::fill(&mut bytes)
            .map_err(|error| LocalFsError::from_io(directory.path(), io::Error::other(error)))?;
        let suffix: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
        let name = format!(".nocterm-{suffix}.guard");
        let file = OpenOptions::default()
            .create_new(true)
            .follow(false)
            .desired_access(FILE_READ_ATTRIBUTES | FILE_WRITE_DATA)
            .open_path_at(directory.file(), &name)
            .map_err(|error| LocalFsError::from_io(directory.path(), error))?;
        let pin = reopen_file(
            file.as_handle(),
            AccessMode::DELETE | AccessMode::FILE_READ_ATTRIBUTES,
            ShareMode::FILE_SHARE_READ | ShareMode::FILE_SHARE_WRITE,
            Flags::FILE_FLAG_OPEN_REPARSE_POINT,
        );
        let guard = match pin {
            Ok(pin) => Self { file: Some(pin) },
            Err(error) => {
                drop(file);
                let _ = OpenOptions::default().unlink_at(directory.file(), &name);
                return Err(LocalFsError::from_io(directory.path(), error));
            }
        };
        drop(file);
        // Every ancestor is now nonempty and pinned. Reject reparse conversion
        // that may have happened while an earlier ancestor was still empty.
        directory.validate_publication_ancestry()?;
        Ok(guard)
    }
    pub(crate) fn close(mut self) -> io::Result<()> {
        self.file
            .take()
            .expect("guard owns its handle")
            .delete_by_handle()
            .map_err(|(_, error)| error)
    }
}
impl Drop for PublicationGuard {
    fn drop(&mut self) {
        if let Some(file) = self.file.take() {
            let _ = file.delete_by_handle();
        }
    }
}
