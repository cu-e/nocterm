use nocterm_core::persist;
use nocterm_snippets::{Library, SnippetRepository, SnippetStorageError, StorageOperation};
use std::path::PathBuf;
pub struct FileSnippetRepository {
    path: PathBuf,
}
impl FileSnippetRepository {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
}
impl SnippetRepository for FileSnippetRepository {
    fn load(&self) -> Result<Option<Library>, SnippetStorageError> {
        persist::load(&self.path)
            .map_err(|error| SnippetStorageError::new(StorageOperation::Read, error))
    }
    fn save(&self, library: &Library) -> Result<(), SnippetStorageError> {
        persist::save(&self.path, library)
            .map_err(|error| SnippetStorageError::new(StorageOperation::Write, error))
    }
}
