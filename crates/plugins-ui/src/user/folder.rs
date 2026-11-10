//! The plugins folder: where the user keeps plugin files to install, which the section opens in
//! the file manager and lists for install. A file there is only a candidate: it is installed
//! through the same consent as a file picked by hand, and nothing in the folder ever loads.

use std::{path::PathBuf, sync::Arc};

use gpui_kit::SharedString;
use nocterm_core::error::SharedCause;

/// Most files of the folder the section lists.
pub const MAX_LISTED: usize = 20;

/// The folder the user keeps plugin files in. Named for the User section's need; the
/// application implements it over a directory beside its settings. Both calls touch the disk,
/// so the section runs them off the UI thread.
pub trait PluginsFolder: Send + Sync + 'static {
    /// Creates the folder if it is missing and gives its path.
    fn prepare(&self) -> Result<PathBuf, FolderError>;

    /// The plugin files directly in the folder (`.toml` and `.wasm` regular files, never
    /// through a link), newest first, at most [`MAX_LISTED`]. A missing folder holds none.
    fn packages(&self) -> Result<Vec<PathBuf>, FolderError>;
}

/// The folder could not be created or read.
#[derive(Clone, Debug, thiserror::Error)]
#[error("The plugins folder could not be opened")]
pub struct FolderError(#[source] SharedCause);

impl FolderError {
    pub fn new(cause: SharedCause) -> Self {
        Self(cause)
    }
}

/// The folder, the files the section last found in it and why it last failed.
pub(super) struct FolderState {
    folder: Arc<dyn PluginsFolder>,
    found: Vec<PathBuf>,
    error: Option<SharedString>,
}

impl FolderState {
    pub(super) fn new(folder: Arc<dyn PluginsFolder>) -> Self {
        Self {
            folder,
            found: Vec::new(),
            error: None,
        }
    }

    pub(super) fn found(&self) -> &[PathBuf] {
        &self.found
    }

    pub(super) fn error(&self) -> Option<&SharedString> {
        self.error.as_ref()
    }

    pub(super) fn port(&self) -> Arc<dyn PluginsFolder> {
        self.folder.clone()
    }

    /// What a listing found, or why it failed.
    pub(super) fn listed(&mut self, listed: Result<Vec<PathBuf>, FolderError>) {
        match listed {
            Ok(found) => {
                self.found = found;
                self.error = None;
            }
            Err(error) => self.error = Some(super::port::describe(&error).into()),
        }
    }

    pub(super) fn failed(&mut self, error: &FolderError) {
        self.error = Some(super::port::describe(error).into());
    }
}
