//! Physical discovery supplies deterministic documents to the pure catalog builder.
use nocterm_themes::{
    CatalogBuilder, FILE_LIMIT, ThemeCatalog, ThemeDirs, ThemeOperationError, ThemeSource, valid_id,
};
use std::{
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
};
pub(crate) fn scan(dirs: &ThemeDirs) -> ThemeCatalog {
    let mut discovery = Discovery {
        catalog: CatalogBuilder::new(),
    };
    for path in discovery.paths(&dirs.user) {
        discovery.document(&path, ThemeSource::User(path.clone()));
    }
    for directory in discovery.paths(&dirs.installed) {
        let Some(id) = directory
            .file_name()
            .and_then(|x| x.to_str())
            .filter(|x| valid_id(x))
        else {
            continue;
        };
        match fs::symlink_metadata(&directory) {
            Ok(m) if m.is_dir() && !m.file_type().is_symlink() => {}
            Ok(_) => continue,
            Err(e) => {
                discovery
                    .catalog
                    .problem(&directory, fs_error(&directory, "inspect pack", e));
                continue;
            }
        }
        let manifest_path = directory.join("extension.toml");
        match read_bounded(&manifest_path) {
            Ok(bytes) => {
                if let Some(source) = discovery.catalog.manifest(&manifest_path, id, &bytes) {
                    for path in discovery.paths(&directory.join("themes")) {
                        discovery.document(&path, source.clone());
                    }
                }
            }
            Err(error) => discovery.catalog.problem(&manifest_path, error),
        }
    }
    discovery.catalog.finish()
}
struct Discovery {
    catalog: CatalogBuilder,
}
impl Discovery {
    fn paths(&mut self, directory: &Path) -> Vec<PathBuf> {
        match fs::read_dir(directory) {
            Ok(entries) => {
                let mut paths = Vec::new();
                for entry in entries {
                    match entry {
                        Ok(e) => paths.push(e.path()),
                        Err(e) => self
                            .catalog
                            .problem(directory, fs_error(directory, "enumerate", e)),
                    }
                }
                paths.sort();
                paths
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Vec::new(),
            Err(e) => {
                self.catalog
                    .problem(directory, fs_error(directory, "read directory", e));
                Vec::new()
            }
        }
    }
    fn document(&mut self, path: &Path, source: ThemeSource) {
        if path.extension().is_none_or(|x| x != "json") {
            return;
        }
        match fs::symlink_metadata(path) {
            Ok(m) if m.is_file() && !m.file_type().is_symlink() => {}
            Ok(_) => return,
            Err(e) => {
                self.catalog
                    .problem(path, fs_error(path, "inspect original", e));
                return;
            }
        }
        match read_bounded(path) {
            Ok(bytes) => self.catalog.document(path, source, &bytes),
            Err(error) => self.catalog.problem(path, error),
        }
    }
}
fn read_bounded(path: &Path) -> Result<Vec<u8>, ThemeOperationError> {
    let metadata = fs::symlink_metadata(path).map_err(|e| fs_error(path, "inspect", e))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(ThemeOperationError::NotPlain(path.into()));
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(|e| fs_error(path, "open", e))?
        .take(FILE_LIMIT as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| fs_error(path, "read", e))?;
    if bytes.len() > FILE_LIMIT {
        return Err(ThemeOperationError::FileLimit);
    }
    Ok(bytes)
}
fn fs_error(path: &Path, operation: &'static str, source: io::Error) -> ThemeOperationError {
    ThemeOperationError::Filesystem {
        operation,
        path: path.into(),
        source,
    }
}
#[cfg(test)]
mod tests;
