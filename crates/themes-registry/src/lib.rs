//! Physical Zed registry, bounded discovery and staged pack publication.
//!
//! Owns: HTTP, local scans, archive expansion and directory transactions.
//! Does not own: palettes, catalog rules, scheduling, selection or views.
//! Ports provided: ThemeRegistry, ThemeLibrary. Ports used: none.
//! Settings section: none. Globals: none.
//! Invariants: validate before publication; rollback preserves the previous pack.
mod archive;
mod discovery;
mod publication;
mod registry;
use nocterm_themes::{
    ExtensionInfo, ThemeCatalog, ThemeChange, ThemeDirs, ThemeLibrary, ThemeMutation,
    ThemeOperationError, ThemeSource,
};
pub use registry::ZedRegistry;
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};
/// A single backend serializes mutations independently of view lifetime.
pub struct LocalThemeLibrary {
    dirs: ThemeDirs,
    mutation: Mutex<Option<Arc<ZedRegistry>>>,
}
impl LocalThemeLibrary {
    pub fn new(dirs: ThemeDirs, registry: Option<Arc<ZedRegistry>>) -> Self {
        Self {
            dirs,
            mutation: Mutex::new(registry),
        }
    }
}
impl ThemeLibrary for LocalThemeLibrary {
    fn scan(&self) -> ThemeCatalog {
        discovery::scan(&self.dirs)
    }
    fn install(&self, extension: &ExtensionInfo) -> Result<ThemeChange, ThemeOperationError> {
        let guard = self.mutation.lock().unwrap_or_else(|e| e.into_inner());
        let registry = guard.as_ref().ok_or(ThemeOperationError::Unsupported)?;
        let bytes = registry.download(extension)?;
        archive::install_archive(&self.dirs.installed, extension, &bytes)?;
        Ok(ThemeChange {
            operation: ThemeMutation::Install(extension.id.clone()),
            catalog: self.scan(),
            removed: Vec::new(),
        })
    }
    fn uninstall(&self, id: &str) -> Result<ThemeChange, ThemeOperationError> {
        let _guard = self.mutation.lock().unwrap_or_else(|e| e.into_inner());
        let before = self.scan();
        archive::uninstall(&self.dirs.installed, id)?;
        let catalog = self.scan();
        let removed = before
            .entries()
            .iter()
            .filter(|e| matches!(&e.source,ThemeSource::Installed{id:pack,..} if pack == id))
            .map(|e| (e.name.clone(), e.appearance))
            .collect();
        Ok(ThemeChange {
            operation: ThemeMutation::Uninstall(id.into()),
            catalog,
            removed,
        })
    }
    fn prepare_user_directory(&self) -> Result<PathBuf, ThemeOperationError> {
        archive::plain_directory(&self.dirs.user)?;
        Ok(self.dirs.user.clone())
    }
}
