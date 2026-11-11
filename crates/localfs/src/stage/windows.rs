//! Atomic publication under pinned ancestry and a nonempty, protected leaf.
use crate::{
    directory::{DirectoryCapability, check_name},
    windows::PublicationGuard,
};
use nocterm_session::{fs::UploadMode, local_fs::*};
use std::{ffi::OsStr, io::Write};

struct Stage {
    directory: DirectoryCapability,
    file: Option<tempfile::NamedTempFile>,
    guard: Option<PublicationGuard>,
    synced: bool,
    committed: bool,
}
pub(crate) fn create(directory: DirectoryCapability) -> LocalResult<Box<dyn LocalStage>> {
    let guard = PublicationGuard::new(&directory)?;
    let file = tempfile::Builder::new()
        .prefix(".nocterm-")
        .suffix(".part")
        .tempfile_in(directory.path())
        .map_err(|error| LocalFsError::from_io(directory.path(), error))?;
    Ok(Box::new(Stage {
        directory,
        file: Some(file),
        guard: Some(guard),
        synced: false,
        committed: false,
    }))
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
        self.file
            .as_mut()
            .map(tempfile::NamedTempFile::as_file_mut)
            .ok_or(LocalFsError::Unsupported("a consumed stage"))
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
        let target = self.directory.path().join(name);
        let file = self
            .file
            .take()
            .ok_or(LocalFsError::Unsupported("a consumed stage"))?;
        let result = if mode == UploadMode::Replace {
            file.persist(&target)
        } else {
            file.persist_noclobber(&target)
        };
        if let Err(error) = result {
            self.file = Some(error.file);
            return Err(LocalFsError::from_io(target, error.error));
        }
        self.committed = true;
        // After rename committed, cleanup failure is a warning, never a failed save.
        if let Some(guard) = self.guard.take()
            && let Err(error) = guard.close()
        {
            return Ok(PublicationOutcome::PublishedWithWarning(
                DurabilityWarning::new(error),
            ));
        }
        Ok(PublicationOutcome::Published)
    }
    fn cancel(mut self: Box<Self>) -> LocalResult<()> {
        self.writable()?;
        let file = self
            .file
            .take()
            .ok_or(LocalFsError::Unsupported("a consumed stage"))?;
        let path = file.path().to_owned();
        file.close()
            .map_err(|error| LocalFsError::from_io(path, error))?;
        if let Some(guard) = self.guard.take() {
            guard
                .close()
                .map_err(|error| LocalFsError::from_io(self.directory.path(), error))?;
        }
        Ok(())
    }
}
impl Drop for Stage {
    fn drop(&mut self) {
        // Cleanup must finish before either the guard or pinned ancestry drops.
        if let Some(file) = self.file.take() {
            let _ = file.close();
        }
    }
}
