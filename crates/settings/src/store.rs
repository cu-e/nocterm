//! `settings.toml` on disk.

use std::path::{Path, PathBuf};

use nocterm_core::{PersistError, persist};

use crate::SettingsDocument;

/// The user's settings file.
///
/// The file holds only what differs from the defaults, so a default that
/// improves in a later release reaches everyone who never touched it.
#[derive(Debug, Clone)]
pub struct SettingsFile {
    file: persist::RevisionedFile,
}

impl SettingsFile {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            file: persist::RevisionedFile::new(path),
        }
    }

    pub fn path(&self) -> &Path {
        self.file.path()
    }

    /// Reads the file's tables. A file that does not exist yields the
    /// defaults. Only a file that is not TOML at all, or cannot be read, is
    /// an error; each section is checked when it is registered.
    pub fn load(&self) -> Result<SettingsDocument, PersistError> {
        let table = self.file.load::<toml::Table>()?.unwrap_or_default();
        Ok(SettingsDocument::from_table(table))
    }

    /// Writes the settings, keeping the comments already in the file.
    ///
    /// A table the document does not write from a section, because it could
    /// not be read or nothing claims it, is left as the file now has it.
    pub fn save(&self, settings: &SettingsDocument) -> Result<(), PersistError> {
        self.file
            .save_preserving_owned(&settings.to_table(), |key| settings.owns(key))
    }
}

#[cfg(test)]
#[path = "store/tests.rs"]
mod tests;
