//! A package store that keeps what it publishes in memory, for a developer session.

use std::{
    collections::BTreeMap,
    path::Path,
    sync::{Mutex, MutexGuard},
};

use nocterm_plugin_api::{ContentHash, Package, PackageError, PluginPackages};
use nocterm_plugin_model::PluginId;

use crate::{parse::sha256, store::read_package};

/// Reads packages from disk like [`PackageStore`], but publishes, loads and removes them in
/// memory only: no package directory is ever created, so a developer session leaves nothing
/// behind.
pub struct SessionStore {
    reserved_names: Vec<String>,
    held: Mutex<BTreeMap<PluginId, Package>>,
}

impl SessionStore {
    /// `reserved_names` are the names a package may not take, as for [`PackageStore::new`].
    pub fn new(reserved_names: Vec<String>) -> Self {
        Self {
            reserved_names,
            held: Mutex::new(BTreeMap::new()),
        }
    }

    fn held(&self) -> MutexGuard<'_, BTreeMap<PluginId, Package>> {
        self.held
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl PluginPackages for SessionStore {
    fn read(&self, location: &str) -> Result<Package, PackageError> {
        let names: Vec<&str> = self.reserved_names.iter().map(String::as_str).collect();
        read_package(Path::new(location), &names)
    }

    fn publish(&self, package: &Package, replace: bool) -> Result<(), PackageError> {
        if sha256(package.bytes()) != *package.hash() {
            return Err(PackageError::HashChanged);
        }
        let id = package.manifest().plugin().id();
        if !id.is_user() {
            return Err(PackageError::NotUserId(id.clone()));
        }
        let mut held = self.held();
        if !replace && held.contains_key(id) {
            return Err(PackageError::Duplicate(id.clone()));
        }
        held.insert(id.clone(), package.clone());
        Ok(())
    }

    fn load(&self, plugin: &PluginId, expected: &ContentHash) -> Result<Package, PackageError> {
        match self.held().get(plugin) {
            Some(package) if package.hash() == expected => Ok(package.clone()),
            Some(_) => Err(PackageError::HashChanged),
            None => Err(PackageError::NotFound),
        }
    }

    fn remove(&self, plugin: &PluginId) -> Result<(), PackageError> {
        self.held().remove(plugin);
        Ok(())
    }
}
