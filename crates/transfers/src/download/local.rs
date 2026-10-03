//! Held directory capabilities prevent ancestor-link races on Unix. The safe
//! rustix wrapper owns descriptors; publication and cleanup use relative names.
use crate::local_error;
use nocterm_session::FsError;
#[cfg(unix)]
use rustix::fs::{self, AtFlags, Mode, OFlags};
#[cfg(unix)]
use std::sync::Arc;
use std::{
    fs::File,
    io,
    path::{Path, PathBuf},
};

#[derive(Clone)]
pub(super) struct LocalDirectory {
    path: PathBuf,
    #[cfg(unix)]
    handle: Arc<File>,
}
impl LocalDirectory {
    pub(super) fn path(&self) -> &Path {
        &self.path
    }
    pub(super) fn open(path: &Path) -> Result<Self, FsError> {
        #[cfg(unix)]
        {
            use std::path::Component;
            let root = fs::open(
                "/",
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(|e| local_error(path, e.into()))?;
            let mut dir = Self {
                path: PathBuf::from("/"),
                handle: Arc::new(root.into()),
            };
            for component in path.components() {
                match component {
                    Component::RootDir => {}
                    Component::Normal(name) => {
                        dir = dir
                            .open_child(name, false)
                            .map_err(|e| local_error(path, e))?;
                    }
                    _ => {
                        return Err(FsError::Other(
                            "destination must be absolute without parent components".into(),
                        ));
                    }
                }
            }
            Ok(dir)
        }
        #[cfg(not(unix))]
        {
            validate_directory(path).map_err(|e| local_error(path, e))?;
            Ok(Self {
                path: path.to_owned(),
            })
        }
    }
    pub(super) fn child(&self, name: &str) -> Result<Self, FsError> {
        #[cfg(unix)]
        {
            self.open_child(std::ffi::OsStr::new(name), true)
                .map_err(|e| local_error(&self.path.join(name), e))
        }
        #[cfg(not(unix))]
        {
            validate_directory(&self.path).map_err(|e| local_error(&self.path, e))?;
            let child = self.path.join(name);
            match std::fs::create_dir(&child) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(local_error(&child, e)),
            }
            Self::open(&child)
        }
    }
    pub(super) fn exists(&self, name: &str) -> Result<bool, FsError> {
        #[cfg(unix)]
        {
            match fs::statat(self.handle.as_ref(), name, AtFlags::SYMLINK_NOFOLLOW) {
                Ok(_) => Ok(true),
                Err(rustix::io::Errno::NOENT) => Ok(false),
                Err(e) => Err(local_error(&self.path, e.into())),
            }
        }
        #[cfg(not(unix))]
        {
            match std::fs::symlink_metadata(self.path.join(name)) {
                Ok(_) => Ok(true),
                Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
                Err(e) => Err(local_error(&self.path, e)),
            }
        }
    }
    pub(super) fn stage(&self) -> Result<StagedFile, FsError> {
        #[cfg(unix)]
        {
            let name = format!(".nocterm-{}.part", uuid::Uuid::new_v4());
            let fd = fs::openat(
                self.handle.as_ref(),
                name.as_str(),
                OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::WRONLY,
                Mode::RUSR | Mode::WUSR,
            )
            .map_err(|e| local_error(&self.path, e.into()))?;
            Ok(StagedFile {
                file: fd.into(),
                directory: self.clone(),
                name,
            })
        }
        #[cfg(not(unix))]
        {
            validate_directory(&self.path).map_err(|e| local_error(&self.path, e))?;
            let file = tempfile::Builder::new()
                .prefix(".nocterm-")
                .suffix(".part")
                .tempfile_in(&self.path)
                .map_err(|e| local_error(&self.path, e))?;
            Ok(StagedFile {
                file: Some(file),
                directory: self.clone(),
            })
        }
    }
    #[cfg(unix)]
    pub(super) fn read_file(&self, name: &std::ffi::OsStr) -> io::Result<File> {
        // A regular file may be replaced with a FIFO between discovery and
        // open. Never wait for a FIFO writer before checking the opened inode.
        let file = fs::openat(
            self.handle.as_ref(),
            name,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
            Mode::empty(),
        )?;
        let metadata = fs::fstat(&file)?;
        if !fs::FileType::from_raw_mode(metadata.st_mode).is_file() {
            return Err(io::Error::other("local source is no longer a regular file"));
        }
        Ok(file.into())
    }
    #[cfg(unix)]
    fn open_child(&self, name: &std::ffi::OsStr, create: bool) -> io::Result<Self> {
        if create {
            match fs::mkdirat(self.handle.as_ref(), name, Mode::from_raw_mode(0o755)) {
                Ok(()) | Err(rustix::io::Errno::EXIST) => {}
                Err(e) => return Err(e.into()),
            }
        }
        let fd = fs::openat(
            self.handle.as_ref(),
            name,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )?;
        Ok(Self {
            path: self.path.join(name),
            handle: Arc::new(fd.into()),
        })
    }
}
#[cfg(not(unix))]
fn validate_directory(path: &Path) -> io::Result<()> {
    let mut current = PathBuf::new();
    for component in path.components() {
        if matches!(component, std::path::Component::ParentDir) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "parent component",
            ));
        }
        current.push(component);
        let metadata = std::fs::symlink_metadata(&current)?;
        if !metadata.is_dir() || metadata.is_symlink() {
            return Err(io::Error::other(
                "destination contains a link or non-directory",
            ));
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes()
                & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
                != 0
            {
                return Err(io::Error::other("destination contains a reparse point"));
            }
        }
    }
    Ok(())
}

pub(super) struct StagedFile {
    #[cfg(unix)]
    file: File,
    #[cfg(unix)]
    name: String,
    #[cfg(not(unix))]
    file: Option<tempfile::NamedTempFile>,
    directory: LocalDirectory,
}
impl StagedFile {
    pub(super) fn file(&self) -> &File {
        #[cfg(unix)]
        {
            &self.file
        }
        #[cfg(not(unix))]
        {
            self.file.as_ref().expect("unpublished temporary").as_file()
        }
    }
    pub(super) fn publish(&mut self, name: &str, replace: bool) -> io::Result<()> {
        #[cfg(unix)]
        {
            let directory = self.directory.handle.as_ref();
            if replace {
                fs::renameat(directory, self.name.as_str(), directory, name)
            } else {
                fs::linkat(
                    directory,
                    self.name.as_str(),
                    directory,
                    name,
                    AtFlags::empty(),
                )
            }?;
            self.directory.handle.sync_all()?;
            Ok(())
        }
        #[cfg(not(unix))]
        {
            validate_directory(&self.directory.path)?;
            let file = self.file.take().expect("unpublished temporary");
            let destination = self.directory.path.join(name);
            match if replace {
                file.persist(destination)
            } else {
                file.persist_noclobber(destination)
            } {
                Ok(_) => Ok(()),
                Err(error) => {
                    self.file = Some(error.file);
                    Err(error.error)
                }
            }
        }
    }
}
#[cfg(unix)]
impl Drop for StagedFile {
    fn drop(&mut self) {
        let _ = fs::unlinkat(
            self.directory.handle.as_ref(),
            self.name.as_str(),
            AtFlags::empty(),
        );
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::{ffi::OsStr, io::Read, time::Duration};

    #[test]
    fn fifo_replacement_never_waits_for_a_writer_and_regular_file_still_reads() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("selected");
        std::fs::write(&source, b"selected regular file").unwrap();
        assert!(std::fs::symlink_metadata(&source).unwrap().is_file());
        let directory = LocalDirectory::open(root.path()).unwrap();
        // The inode changes after successful regular-file discovery.
        std::fs::remove_file(&source).unwrap();
        fs::mkfifoat(
            directory.handle.as_ref(),
            "selected",
            Mode::RUSR | Mode::WUSR,
        )
        .unwrap();
        let worker_directory = directory.clone();
        let (send, receive) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let result = worker_directory.read_file(OsStr::new("selected"));
            let _ = send.send(result.map(|_| ()));
        });
        let result = receive.recv_timeout(Duration::from_secs(1));
        if result.is_err() {
            // Rescue the old blocking implementation so a failed regression
            // does not leave a worker stuck until the test process exits.
            let rescue = fs::openat(
                directory.handle.as_ref(),
                "selected",
                OFlags::WRONLY | OFlags::NONBLOCK | OFlags::CLOEXEC,
                Mode::empty(),
            );
            if rescue.is_ok() {
                worker.join().unwrap();
            }
            panic!("opening the replaced FIFO blocked waiting for a writer");
        }
        assert!(
            result.unwrap().is_err(),
            "a FIFO cannot be uploaded as a regular file"
        );
        worker.join().unwrap();
        std::fs::remove_file(&source).unwrap();
        std::fs::write(&source, b"regular contents").unwrap();
        let mut regular = directory.read_file(OsStr::new("selected")).unwrap();
        let mut bytes = Vec::new();
        regular.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, b"regular contents");
    }
}
