use std::{io, path::PathBuf};

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
        #[cfg(not(any(unix, windows)))]
        return Err(self.read_error(io::Error::new(
            io::ErrorKind::Unsupported,
            "no non-following catalog reader for this platform",
        )));
        #[cfg(any(unix, windows))]
        match nocterm_core::fs::read_regular_bounded(&self.path, MAX_CATALOG_BYTES) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(nocterm_core::fs::BoundedReadError::TooLarge { .. }) => {
                Err(CatalogError::TooLarge.into())
            }
            Err(nocterm_core::fs::BoundedReadError::NotRegular { .. }) => {
                Err(self.read_error(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "project catalog must be a regular file",
                )))
            }
            Err(nocterm_core::fs::BoundedReadError::Io { source, .. })
                if source.kind() == io::ErrorKind::NotFound =>
            {
                Ok(None)
            }
            Err(nocterm_core::fs::BoundedReadError::Io { source, .. }) => {
                Err(self.read_error(source))
            }
        }
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
