use std::{
    fs::File,
    io::{self, Read as _},
    path::{Path, PathBuf},
};

use nocterm_project_model::{
    Catalog, CatalogError, MAX_CATALOG_BYTES, ProjectRepository, ProjectStorageError,
};

/// An exact private catalog path under ancestors trusted by application setup.
///
/// Reads refuse final symlinks. Saves deliberately replace the final path using
/// core's private atomic publication; they never follow its existing symlink.
pub struct FileProjectRepository {
    path: PathBuf,
}

impl FileProjectRepository {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    fn read(&self) -> Result<Option<Vec<u8>>, ProjectStorageError> {
        let file = match open_catalog(&self.path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(self.read_error(error)),
        };
        let metadata = file.metadata().map_err(|error| self.read_error(error))?;
        if !metadata.is_file() {
            return Err(self.read_error(io::Error::new(
                io::ErrorKind::InvalidInput,
                "project catalog must be a regular file",
            )));
        }
        if metadata.len() > MAX_CATALOG_BYTES as u64 {
            return Err(CatalogError::TooLarge.into());
        }
        let mut bytes = Vec::new();
        file.take(MAX_CATALOG_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| self.read_error(error))?;
        if bytes.len() > MAX_CATALOG_BYTES {
            return Err(CatalogError::TooLarge.into());
        }
        Ok(Some(bytes))
    }

    fn read_error(&self, error: io::Error) -> ProjectStorageError {
        ProjectStorageError::read(self.path.clone(), error)
    }
}

impl ProjectRepository for FileProjectRepository {
    fn load(&self) -> Result<Option<Catalog>, ProjectStorageError> {
        self.read()?
            .map(|bytes| Catalog::decode(&bytes).map_err(Into::into))
            .transpose()
    }

    fn save(&self, catalog: &Catalog) -> Result<(), ProjectStorageError> {
        let bytes = catalog.encode()?;
        nocterm_core::fs::write_private_atomic(&self.path, &bytes)
            .map_err(|error| ProjectStorageError::publish(self.path.clone(), error))
    }
}

#[cfg(unix)]
fn open_catalog(path: &Path) -> io::Result<File> {
    use rustix::fs::{CWD, Mode, OFlags};
    rustix::fs::openat(
        CWD,
        path,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK,
        Mode::empty(),
    )
    .map(File::from)
    .map_err(Into::into)
}

#[cfg(windows)]
fn open_catalog(path: &Path) -> io::Result<File> {
    use std::{fs::OpenOptions, os::windows::fs::OpenOptionsExt as _};
    // Win32 FILE_FLAG_OPEN_REPARSE_POINT: inspect the final link itself.
    const OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    OpenOptions::new()
        .read(true)
        .custom_flags(OPEN_REPARSE_POINT)
        .open(path)
}

#[cfg(not(any(unix, windows)))]
fn open_catalog(_path: &Path) -> io::Result<File> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "no non-following catalog reader for this platform",
    ))
}
