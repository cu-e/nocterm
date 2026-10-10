use crate::directory::{DirectoryCapability, check_name, metadata_native, parent};
use nocterm_session::{FileMetadata, local_fs::*};
use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};

struct Source {
    file: File,
    path: PathBuf,
}
pub(crate) fn open(path: &Path) -> LocalResult<Box<dyn LocalSource>> {
    let (directory, name) = parent(path)?;
    open_relative(&directory, name)
}
pub(crate) fn open_relative(
    directory: &DirectoryCapability,
    name: &std::ffi::OsStr,
) -> LocalResult<Box<dyn LocalSource>> {
    check_name(name)?;
    let location = directory.path().join(name);
    let path = location.as_path();
    #[cfg(unix)]
    let file: File = {
        use rustix::fs::{self, Mode, OFlags};
        // Never wait for a writer if a selected regular file was swapped to FIFO.
        let fd = fs::openat(
            directory.file(),
            name,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
            Mode::empty(),
        )
        .map_err(|error| {
            if error == rustix::io::Errno::LOOP {
                LocalFsError::LinkRefused(path.to_owned())
            } else {
                LocalFsError::from_io(path, error.into())
            }
        })?;
        fd.into()
    };
    #[cfg(not(unix))]
    let file = {
        if directory.inspect(name)?.is_none_or(|metadata| {
            metadata.is_symlink || metadata.kind != nocterm_session::EntryKind::File
        }) {
            return Err(LocalFsError::NotRegular(path.to_owned()));
        }
        let mut options = std::fs::OpenOptions::new();
        options.read(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.custom_flags(
                windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT,
            );
        }
        options
            .open(path)
            .map_err(|error| LocalFsError::from_io(path, error))?
    };
    let metadata = file
        .metadata()
        .map_err(|error| LocalFsError::from_io(path, error))?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes()
            & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
            != 0
        {
            return Err(LocalFsError::LinkRefused(path.to_owned()));
        }
    }
    if !metadata.is_file() {
        return Err(LocalFsError::NotRegular(path.to_owned()));
    }
    Ok(Box::new(Source {
        file,
        path: path.to_owned(),
    }))
}
impl LocalSource for Source {
    fn metadata(&self) -> LocalResult<FileMetadata> {
        self.file
            .metadata()
            .map(|metadata| metadata_native(&metadata))
            .map_err(|error| LocalFsError::from_io(&self.path, error))
    }
    fn read(&mut self, max_bytes: usize) -> LocalResult<Vec<u8>> {
        if max_bytes == 0 || max_bytes > 32 * 1024 {
            return Err(LocalFsError::BoundExceeded("source chunk"));
        }
        let mut bytes = vec![0; max_bytes];
        let count = self
            .file
            .read(&mut bytes)
            .map_err(|error| LocalFsError::from_io(&self.path, error))?;
        bytes.truncate(count);
        Ok(bytes)
    }
    fn close(self: Box<Self>) -> LocalResult<()> {
        Ok(())
    }
}
