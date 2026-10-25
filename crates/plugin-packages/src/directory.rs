//! Reading every package of a directory without installing any: what `nocterm plugin check`
//! shows an author.

use std::{
    ffi::OsStr,
    fs, io,
    path::{Path, PathBuf},
};

use nocterm_plugin_api::{Package, PackageError};

use crate::{parse::parse_package, store::PackageStore};

/// Most files of a directory that are read in one check.
pub const MAX_CHECKED_FILES: usize = 256;

/// One top-level `.toml` file of a directory and what reading it gave.
#[derive(Debug)]
pub struct CheckedFile {
    pub path: PathBuf,
    pub package: Result<Package, PackageError>,
}

/// Why a directory could not be listed.
#[derive(Debug, thiserror::Error)]
pub enum DirectoryError {
    #[error("{path}: {source}")]
    Unreadable {
        path: String,
        #[source]
        source: io::Error,
    },
    #[error("{path} holds more than {max} `.toml` files")]
    TooMany { path: String, max: usize },
}

/// Reads the `.toml` files directly inside `directory`, in name order, exactly as an install
/// reads a file: bounded, without following a link, and validated against `reserved_names`. A
/// link or another file that is not a regular file is reported for that file, not followed.
/// Read-only: no package store is involved and nothing is stored.
pub fn read_directory(
    directory: &Path,
    reserved_names: &[&str],
) -> Result<Vec<CheckedFile>, DirectoryError> {
    let unreadable = |source| DirectoryError::Unreadable {
        path: directory.display().to_string(),
        source,
    };
    let mut paths = Vec::new();
    for entry in fs::read_dir(directory).map_err(unreadable)? {
        let path = entry.map_err(unreadable)?.path();
        if path.extension() == Some(OsStr::new("toml")) {
            paths.push(path);
        }
        if paths.len() > MAX_CHECKED_FILES {
            return Err(DirectoryError::TooMany {
                path: directory.display().to_string(),
                max: MAX_CHECKED_FILES,
            });
        }
    }
    paths.sort();
    Ok(paths
        .into_iter()
        .map(|path| CheckedFile {
            package: PackageStore::read_bytes(&path)
                .and_then(|bytes| parse_package(bytes, reserved_names)),
            path,
        })
        .collect())
}
