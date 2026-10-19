use crate::directory::{DirectoryCapability, check_name};
use nocterm_session::{fs::UploadMode, local_fs::*};
#[cfg(unix)]
use rustix::fs::{self, AtFlags, Mode, OFlags};
#[cfg(unix)]
use std::ffi::OsString;
use std::{ffi::OsStr, io::Write};

struct Stage {
    directory: DirectoryCapability,
    #[cfg(unix)]
    file: std::fs::File,
    #[cfg(unix)]
    name: Option<OsString>,
    #[cfg(not(unix))]
    file: Option<tempfile::NamedTempFile>,
    synced: bool,
    committed: bool,
}
pub(crate) fn create(directory: DirectoryCapability) -> LocalResult<Box<dyn LocalStage>> {
    #[cfg(unix)]
    for _ in 0..100 {
        let mut bytes = [0; 16];
        getrandom::fill(&mut bytes).map_err(|error| {
            LocalFsError::from_io(directory.path(), std::io::Error::other(error))
        })?;
        let name = format!(
            ".nocterm-{}.part",
            bytes
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        );
        let fd = match fs::openat(
            directory.file(),
            name.as_str(),
            OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::WRONLY,
            Mode::RUSR | Mode::WUSR,
        ) {
            Ok(fd) => fd,
            Err(rustix::io::Errno::EXIST) => continue,
            Err(error) => return Err(LocalFsError::from_io(directory.path(), error.into())),
        };
        return Ok(Box::new(Stage {
            directory,
            file: fd.into(),
            name: Some(name.into()),
            synced: false,
            committed: false,
        }));
    }
    #[cfg(unix)]
    return Err(LocalFsError::BoundExceeded(
        "exclusive staging-name attempts",
    ));
    #[cfg(not(unix))]
    {
        super::directory::DirectoryCapability::strict(directory.path())?;
        let file = tempfile::Builder::new()
            .prefix(".nocterm-")
            .suffix(".part")
            .tempfile_in(directory.path())
            .map_err(|error| LocalFsError::from_io(directory.path(), error))?;
        Ok(Box::new(Stage {
            directory,
            file: Some(file),
            synced: false,
            committed: false,
        }))
    }
}
impl Stage {
    fn writable(&self) -> LocalResult<()> {
        if self.committed {
            Err(LocalFsError::Unsupported("a published stage"))
        } else {
            Ok(())
        }
    }
    fn file_mut(&mut self) -> LocalResult<&mut std::fs::File> {
        #[cfg(unix)]
        {
            Ok(&mut self.file)
        }
        #[cfg(not(unix))]
        {
            self.file
                .as_mut()
                .map(tempfile::NamedTempFile::as_file_mut)
                .ok_or(LocalFsError::Unsupported("a consumed stage"))
        }
    }
    fn commit(&mut self, name: &OsStr, mode: UploadMode) -> LocalResult<Option<DurabilityWarning>> {
        #[cfg(unix)]
        {
            let directory = self.directory.file();
            let temporary = self
                .name
                .as_deref()
                .ok_or(LocalFsError::Unsupported("a consumed stage"))?;
            let result = if mode == UploadMode::Replace {
                fs::renameat(directory, temporary, directory, name)
            } else {
                fs::linkat(directory, temporary, directory, name, AtFlags::empty())
            };
            result.map_err(|error| {
                LocalFsError::from_io(self.directory.path().join(name), error.into())
            })?;
            self.committed = true;
            let cleanup = if mode == UploadMode::Create {
                fs::unlinkat(directory, temporary, AtFlags::empty())
                    .err()
                    .map(|error| DurabilityWarning::new(error.into()))
            } else {
                None
            };
            // Publication consumes ownership. Drop cannot delete another file
            // subsequently created under the former staging name.
            self.name = None;
            Ok(cleanup)
        }
        #[cfg(not(unix))]
        {
            super::directory::DirectoryCapability::strict(self.directory.path())?;
            let file = self
                .file
                .take()
                .ok_or(LocalFsError::Unsupported("a consumed stage"))?;
            let target = self.directory.path().join(name);
            let result = if mode == UploadMode::Replace {
                file.persist(target)
            } else {
                file.persist_noclobber(target)
            };
            match result {
                Ok(_) => {
                    self.committed = true;
                    Ok(None)
                }
                Err(error) => {
                    self.file = Some(error.file);
                    Err(LocalFsError::from_io(
                        self.directory.path().join(name),
                        error.error,
                    ))
                }
            }
        }
    }
}
impl LocalStage for Stage {
    fn write(&mut self, bytes: &[u8]) -> LocalResult<()> {
        self.writable()?;
        if bytes.len() > 32 * 1024 {
            return Err(LocalFsError::BoundExceeded("stage chunk"));
        }
        let path = self.directory.path().to_owned();
        self.file_mut()?
            .write_all(bytes)
            .map_err(|error| LocalFsError::from_io(path, error))?;
        self.synced = false;
        Ok(())
    }
    fn sync(&mut self) -> LocalResult<()> {
        self.writable()?;
        let path = self.directory.path().to_owned();
        self.file_mut()?
            .sync_all()
            .map_err(|error| LocalFsError::from_io(path, error))?;
        self.synced = true;
        Ok(())
    }
    fn publish(&mut self, name: &OsStr, mode: UploadMode) -> LocalResult<PublicationOutcome> {
        self.writable()?;
        check_name(name)?;
        if !self.synced {
            self.sync()?;
        }
        let cleanup = self.commit(name, mode)?;
        #[cfg(unix)]
        if let Err(error) = self.directory.file().sync_all() {
            return Ok(PublicationOutcome::PublishedWithWarning(
                DurabilityWarning::new(error),
            ));
        }
        if let Some(warning) = cleanup {
            return Ok(PublicationOutcome::PublishedWithWarning(warning));
        }
        Ok(PublicationOutcome::Published)
    }
    fn cancel(mut self: Box<Self>) -> LocalResult<()> {
        self.writable()?;
        #[cfg(unix)]
        {
            let name = self
                .name
                .as_deref()
                .ok_or(LocalFsError::Unsupported("a consumed stage"))?;
            match fs::unlinkat(self.directory.file(), name, AtFlags::empty()) {
                Ok(()) | Err(rustix::io::Errno::NOENT) => {
                    // A removed/absent candidate relinquishes its name before
                    // Drop, so a subsequent creator cannot be deleted.
                    self.name = None;
                    Ok(())
                }
                Err(error) => Err(LocalFsError::from_io(
                    self.directory.path().join(name),
                    error.into(),
                )),
            }
        }
        #[cfg(not(unix))]
        {
            let file = self
                .file
                .take()
                .ok_or(LocalFsError::Unsupported("a consumed stage"))?;
            let path = file.path().to_owned();
            match file.close() {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(LocalFsError::from_io(path, error)),
            }
        }
    }
}
#[cfg(unix)]
impl Drop for Stage {
    fn drop(&mut self) {
        if let Some(name) = &self.name {
            let _ = fs::unlinkat(self.directory.file(), name, AtFlags::empty());
        }
    }
}
