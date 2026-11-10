//! The plugins folder of the installation: a directory beside the settings where the user keeps
//! plugin files. The User section opens it and offers its files for install; nothing in it loads.
use std::{
    fs,
    path::{Path, PathBuf},
    time::SystemTime,
};

use nocterm_core::error::SharedCause;
use nocterm_plugins_ui::{FolderError, MAX_LISTED, PluginsFolder};

/// The name of the plugins folder in the configuration directory.
pub(crate) const FOLDER_NAME: &str = "user-plugins";

/// The folder at `path`.
pub(crate) struct PluginsDirectory {
    path: PathBuf,
}

impl PluginsDirectory {
    pub(crate) fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

impl PluginsFolder for PluginsDirectory {
    fn prepare(&self) -> Result<PathBuf, FolderError> {
        fs::create_dir_all(&self.path)
            .map(|()| self.path.clone())
            .map_err(|error| FolderError::new(SharedCause::new(error)))
    }

    fn packages(&self) -> Result<Vec<PathBuf>, FolderError> {
        match fs::read_dir(&self.path) {
            Ok(entries) => {
                let mut found: Vec<(SystemTime, PathBuf)> = entries
                    .filter_map(Result::ok)
                    .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
                    .map(|entry| entry.path())
                    .filter(|path| is_package(path))
                    .filter_map(|path| {
                        let modified = fs::symlink_metadata(&path).ok()?.modified().ok()?;
                        Some((modified, path))
                    })
                    .collect();
                found.sort_by(|a, b| b.cmp(a));
                Ok(found
                    .into_iter()
                    .take(MAX_LISTED)
                    .map(|(_, path)| path)
                    .collect())
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(error) => Err(FolderError::new(SharedCause::new(error))),
        }
    }
}

/// A declarative (`.toml`) or code (`.wasm`) package file.
fn is_package(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("toml") || extension.eq_ignore_ascii_case("wasm")
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_folder_lists_package_files_newest_first_and_never_through_a_link() {
        let root = tempfile::tempdir().unwrap();
        let folder = PluginsDirectory::new(root.path().join("plugins"));
        assert!(
            folder.packages().unwrap().is_empty(),
            "a missing folder holds none"
        );
        let path = folder.prepare().unwrap();
        fs::write(path.join("notes.txt"), "x").unwrap();
        fs::write(path.join("clock.toml"), "x").unwrap();
        fs::create_dir(path.join("dir.wasm")).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(path.join("clock.toml"), path.join("link.wasm")).unwrap();
        let listed: Vec<String> = folder
            .packages()
            .unwrap()
            .iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(listed, ["clock.toml"]);
    }
}
