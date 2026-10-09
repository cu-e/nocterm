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
    path: PathBuf,
}

impl SettingsFile {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Reads the file's tables. A file that does not exist yields the
    /// defaults. Only a file that is not TOML at all, or cannot be read, is
    /// an error; each section is checked when it is registered.
    pub fn load(&self) -> Result<SettingsDocument, PersistError> {
        let table = persist::load::<toml::Table>(&self.path)?.unwrap_or_default();
        Ok(SettingsDocument::from_table(table))
    }

    /// Writes the settings, keeping the comments already in the file.
    ///
    /// A table the document does not write from a section, because it could
    /// not be read or nothing claims it, is left as the file now has it.
    pub fn save(&self, settings: &SettingsDocument) -> Result<(), PersistError> {
        let mut table = settings.to_table();
        if let Ok(Some(current)) = persist::load::<toml::Table>(&self.path) {
            table.retain(|key, _| settings.owns(key));
            table.extend(current.into_iter().filter(|(key, _)| !settings.owns(key)));
        }
        persist::save_preserving(&self.path, &table)
    }
}

#[cfg(test)]
#[path = "store/tests.rs"]
mod tests;
