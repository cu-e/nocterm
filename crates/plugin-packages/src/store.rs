//! The package directory: one `<plugin id>.toml` or `<plugin id>.wasm` per installed plugin.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use nocterm_core::fs::{BoundedReadError, read_regular_bounded, write_private_atomic};
use nocterm_plugin_api::{
    Cause, ContentHash, MAX_MODULE_BYTES, Package, PackageError, PackageKind, PluginPackages, kind,
};
use nocterm_plugin_model::PluginId;

use crate::parse::{parse_package, sha256};

/// The kinds of package file, in the order an installed plugin is looked for.
const KINDS: [PackageKind; 2] = [PackageKind::Module, PackageKind::Manifest];

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

    fn path_of(&self, plugin: &PluginId, kind: PackageKind) -> Result<PathBuf, PackageError> {
        if plugin.check().is_err() || !plugin.is_user() {
            return Err(PackageError::NotUserId(plugin.clone()));
        }
        Ok(self.root.join(format!("{plugin}.{}", kind.extension())))
    }

    fn parse(&self, bytes: Vec<u8>) -> Result<Package, PackageError> {
        let names: Vec<&str> = self.reserved_names.iter().map(String::as_str).collect();
        parse_package(bytes, &names)
    }

    /// Reads the bounded bytes of the regular file at `path`, without following a link. The
    /// bound is the largest package of any kind; [`parse_package`] applies the limit of the
    /// kind it finds.
    pub(crate) fn read_bytes(path: &Path) -> Result<Vec<u8>, PackageError> {
        let cap = usize::try_from(MAX_MODULE_BYTES).unwrap_or(usize::MAX);
        read_regular_bounded(path, cap).map_err(|error| match error {
            BoundedReadError::NotRegular { .. } => PackageError::NotRegular,
            BoundedReadError::TooLarge { .. } => PackageError::TooLarge,
            BoundedReadError::Io { source, .. } if source.kind() == io::ErrorKind::NotFound => {
                PackageError::NotFound
            }
            BoundedReadError::Io { source, .. } => unreadable(&source),
        })
    }

    /// Removes the file of `kind`; a file that is not there is not an error.
    fn remove_kind(&self, plugin: &PluginId, kind: PackageKind) -> Result<(), PackageError> {
        match fs::remove_file(self.path_of(plugin, kind)?) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => Err(unwritable(error)),
            _ => Ok(()),
        }
    }
}

/// Reads and checks the package file at `path` exactly as an install does: bounded, without
/// following a link, hashed over the exact bytes and validated against `reserved_names`. Nothing
/// is stored.
pub fn read_package(path: &Path, reserved_names: &[&str]) -> Result<Package, PackageError> {
    parse_package(PackageStore::read_bytes(path)?, reserved_names)
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

    /// Writes the file of the package's kind, then removes the file of the other kind, so an
    /// update that changes the runtime leaves one file.
    fn publish(&self, package: &Package, replace: bool) -> Result<(), PackageError> {
        if sha256(package.bytes()) != *package.hash() {
            return Err(PackageError::HashChanged);
        }
        let id = package.manifest().plugin().id();
        let package_kind = kind(package.bytes());
        let path = self.path_of(id, package_kind)?;
        if !replace {
            for other in KINDS {
                if fs::symlink_metadata(self.path_of(id, other)?).is_ok() {
                    return Err(PackageError::Duplicate(id.clone()));
                }
            }
        }
        write_private_atomic(&path, package.bytes())
            .map_err(|error| unwritable(error.into_io_error()))?;
        for other in KINDS.into_iter().filter(|other| *other != package_kind) {
            self.remove_kind(id, other)?;
        }
        Ok(())
    }

    /// Loads the installed file whose bytes hash to `expected`, whichever kind it is.
    fn load(&self, plugin: &PluginId, expected: &ContentHash) -> Result<Package, PackageError> {
        let mut outcome = PackageError::NotFound;
        for file_kind in KINDS {
            match Self::read_bytes(&self.path_of(plugin, file_kind)?) {
                Ok(bytes) if sha256(&bytes) == *expected => {
                    let package = self.parse(bytes)?;
                    if package.manifest().plugin().id() != plugin {
                        return Err(PackageError::HashChanged);
                    }
                    return Ok(package);
                }
                Ok(_) => outcome = PackageError::HashChanged,
                Err(PackageError::NotFound) => {}
                Err(error) => {
                    if outcome == PackageError::NotFound {
                        outcome = error;
                    }
                }
            }
        }
        Err(outcome)
    }

    fn remove(&self, plugin: &PluginId) -> Result<(), PackageError> {
        let mut outcome = Ok(());
        for file_kind in KINDS {
            if let Err(error) = self.remove_kind(plugin, file_kind) {
                outcome = outcome.and(Err(error));
            }
        }
        outcome
    }
}
