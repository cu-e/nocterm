//! The package directory: one `<plugin id>.toml` per installed plugin.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use nocterm_core::fs::{BoundedReadError, read_regular_bounded, write_private_atomic};
use nocterm_plugin_api::{
    Cause, ContentHash, MAX_PACKAGE_BYTES, Package, PackageError, PluginPackages,
};
use nocterm_plugin_model::PluginId;

use crate::parse::{parse_package, sha256};

/// Packages under one directory.
#[derive(Clone, Debug)]
pub struct PackageStore {
    root: PathBuf,
    reserved_names: Vec<String>,
}

impl PackageStore {
    /// `reserved_names` are the names a package may not take (system plugin names and the
    /// titles of built-in contributions and groups).
    pub fn new(root: PathBuf, reserved_names: Vec<String>) -> Self {
        Self {
            root,
            reserved_names,
        }
    }

    fn path_of(&self, plugin: &PluginId) -> Result<PathBuf, PackageError> {
        if plugin.check().is_err() || !plugin.is_user() {
            return Err(PackageError::NotUserId(plugin.clone()));
        }
        Ok(self.root.join(format!("{plugin}.toml")))
    }

    fn parse(&self, bytes: Vec<u8>) -> Result<Package, PackageError> {
        let names: Vec<&str> = self.reserved_names.iter().map(String::as_str).collect();
        parse_package(bytes, &names)
    }

    /// Reads the bounded bytes of the regular file at `path`, without following a link.
    pub(crate) fn read_bytes(path: &Path) -> Result<Vec<u8>, PackageError> {
        let cap = usize::try_from(MAX_PACKAGE_BYTES).unwrap_or(usize::MAX);
        read_regular_bounded(path, cap).map_err(|error| match error {
            BoundedReadError::NotRegular { .. } => PackageError::NotRegular,
            BoundedReadError::TooLarge { .. } => PackageError::TooLarge,
            BoundedReadError::Io { source, .. } if source.kind() == io::ErrorKind::NotFound => {
                PackageError::NotFound
            }
            BoundedReadError::Io { source, .. } => unreadable(&source),
        })
    }
}

fn unreadable(error: &io::Error) -> PackageError {
    PackageError::Unreadable {
        reason: error.kind().to_string(),
    }
}

/// A failed write or removal, with the I/O error kept as the source.
fn unwritable(error: io::Error) -> PackageError {
    PackageError::Unwritable(Cause::new(error))
}

impl PluginPackages for PackageStore {
    fn read(&self, location: &str) -> Result<Package, PackageError> {
        self.parse(Self::read_bytes(Path::new(location))?)
    }

    fn publish(&self, package: &Package, replace: bool) -> Result<(), PackageError> {
        if sha256(package.bytes()) != *package.hash() {
            return Err(PackageError::HashChanged);
        }
        let id = package.manifest().plugin().id();
        let path = self.path_of(id)?;
        if !replace && fs::symlink_metadata(&path).is_ok() {
            return Err(PackageError::Duplicate(id.clone()));
        }
        write_private_atomic(&path, package.bytes())
            .map_err(|error| unwritable(error.into_io_error()))
    }

    fn load(&self, plugin: &PluginId, expected: &ContentHash) -> Result<Package, PackageError> {
        let bytes = Self::read_bytes(&self.path_of(plugin)?)?;
        if sha256(&bytes) != *expected {
            return Err(PackageError::HashChanged);
        }
        let package = self.parse(bytes)?;
        if package.manifest().plugin().id() != plugin {
            return Err(PackageError::HashChanged);
        }
        Ok(package)
    }

    fn remove(&self, plugin: &PluginId) -> Result<(), PackageError> {
        match fs::remove_file(self.path_of(plugin)?) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => Err(unwritable(error)),
            _ => Ok(()),
        }
    }
}
