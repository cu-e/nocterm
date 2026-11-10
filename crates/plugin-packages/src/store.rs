//! Immutable hash-addressed packages; legacy id-only files remain readable until migration.

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
const MAX_VERSIONS: usize = 8;

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

    fn version_path(
        &self,
        plugin: &PluginId,
        hash: &ContentHash,
        kind: PackageKind,
    ) -> Result<PathBuf, PackageError> {
        self.path_of(plugin, kind)?;
        Ok(self
            .root
            .join(format!("{plugin}.{hash}.{}", kind.extension())))
    }

    fn versions(&self, plugin: &PluginId) -> Result<Vec<PathBuf>, PackageError> {
        self.path_of(plugin, PackageKind::Manifest)?;
        let entries = match fs::read_dir(&self.root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(unwritable(error)),
        };
        let prefix = format!("{plugin}.");
        entries
            .filter_map(|entry| match entry {
                Ok(entry)
                    if entry
                        .file_name()
                        .to_str()
                        .is_some_and(|name| name.starts_with(&prefix)) =>
                {
                    Some(Ok(entry.path()))
                }
                Ok(_) => None,
                Err(error) => Some(Err(unwritable(error))),
            })
            .collect()
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

    /// Publishes immutable bytes under their hash. Existing consent still loads its original
    /// version if the application stops before the new records document commits.
    fn publish(&self, package: &Package, replace: bool) -> Result<(), PackageError> {
        if sha256(package.bytes()) != *package.hash() {
            return Err(PackageError::HashChanged);
        }
        let id = package.manifest().plugin().id();
        let package_kind = kind(package.bytes());
        let path = self.version_path(id, package.hash(), package_kind)?;
        let versions = self.versions(id)?;
        if !replace && !versions.is_empty() {
            return Err(PackageError::Duplicate(id.clone()));
        }
        if !versions.contains(&path) && versions.len() >= MAX_VERSIONS {
            return Err(PackageError::PendingVersions { max: MAX_VERSIONS });
        }
        write_private_atomic(&path, package.bytes())
            .map_err(|error| unwritable(error.into_io_error()))?;
        Ok(())
    }

    /// Loads the installed file whose bytes hash to `expected`, whichever kind it is.
    fn load(&self, plugin: &PluginId, expected: &ContentHash) -> Result<Package, PackageError> {
        let mut outcome = PackageError::NotFound;
        for file_kind in KINDS {
            for path in [
                self.version_path(plugin, expected, file_kind)?,
                self.path_of(plugin, file_kind)?,
            ] {
                match Self::read_bytes(&path) {
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
        }
        if outcome == PackageError::NotFound && !self.versions(plugin)?.is_empty() {
            outcome = PackageError::HashChanged;
        }
        Err(outcome)
    }

    fn remove(&self, plugin: &PluginId) -> Result<(), PackageError> {
        let mut outcome = Ok(());
        for path in self.versions(plugin)? {
            if let Err(error) = fs::remove_file(path).map_err(unwritable) {
                outcome = outcome.and(Err(error));
            }
        }
        outcome
    }

    fn retain(&self, plugin: &PluginId, expected: &ContentHash) -> Result<(), PackageError> {
        // Validate the retained bytes before removing any recovery version.
        let package = self.load(plugin, expected)?;
        let path = self.version_path(plugin, expected, kind(package.bytes()))?;
        let already_published =
            Self::read_bytes(&path).is_ok_and(|bytes| sha256(&bytes) == *expected);
        if !already_published {
            write_private_atomic(&path, package.bytes())
                .map_err(|error| unwritable(error.into_io_error()))?;
        }
        let kept: Vec<_> = KINDS
            .into_iter()
            .map(|kind| self.version_path(plugin, expected, kind))
            .collect::<Result<_, _>>()?;
        for path in self.versions(plugin)? {
            if !kept.contains(&path) {
                fs::remove_file(path).map_err(unwritable)?;
            }
        }
        Ok(())
    }
}
