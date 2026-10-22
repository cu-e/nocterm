use std::sync::Arc;

use nocterm_project_model::{Catalog, ProjectStorageError};

/// Native acquisition happens before installation on the composition executor.
pub struct PreparedProjects {
    catalog: Catalog,
    load_error: Option<Arc<ProjectStorageError>>,
    persistent: bool,
}

impl PreparedProjects {
    pub fn loaded(loaded: Result<Option<Catalog>, ProjectStorageError>, persistent: bool) -> Self {
        let (catalog, load_error) = match loaded {
            Ok(catalog) => (catalog.unwrap_or_default(), None),
            Err(error) => (Catalog::default(), Some(Arc::new(error))),
        };
        Self {
            catalog,
            load_error,
            persistent,
        }
    }

    pub(crate) fn into_parts(self) -> (Catalog, Option<Arc<ProjectStorageError>>, bool) {
        (self.catalog, self.load_error, self.persistent)
    }
}
