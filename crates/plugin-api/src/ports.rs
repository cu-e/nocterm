//! Ports of the user plugin track: where packages come from and how a package's code runs.
//!
//! The api crate only names them. The adapter `nocterm-plugin-packages` implements
//! [`PluginPackages`]; the declarative interpreter in this crate and, later, a code runtime
//! implement [`PluginRuntime`].

use alloc::{boxed::Box, string::String, sync::Arc, vec::Vec};
use core::{error::Error as StdError, fmt};

use nocterm_plugin_model::PluginId;
use thiserror::Error;

use crate::{
    ApiVersion, ContentHash, Effects, Event, LocalId, Manifest, ManifestError, Snapshot, View,
    ViewError,
};

/// The largest package, in bytes.
pub const MAX_PACKAGE_BYTES: u64 = 256 * 1024;

/// A package read from its source: the exact bytes, their hash and the checked manifest.
#[derive(Clone, Debug, PartialEq)]
pub struct Package {
    hash: ContentHash,
    manifest: Manifest,
    bytes: Vec<u8>,
}

impl Package {
    /// Joins what the adapter read. `hash` must be the SHA-256 of `bytes` and `manifest` must
    /// have been validated from those same bytes: nothing here checks either, so only a
    /// package adapter, which hashed and parsed one buffer, builds a `Package`.
    pub fn new(hash: ContentHash, manifest: Manifest, bytes: Vec<u8>) -> Self {
        Self {
            hash,
            manifest,
            bytes,
        }
    }

    pub fn hash(&self) -> &ContentHash {
        &self.hash
    }

    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// The exact bytes that were hashed and parsed.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

/// The error an adapter hit, kept so the chain of causes survives the port.
///
/// It is equal only to itself, because the adapter's error types are not comparable.
#[derive(Clone, Debug)]
pub struct Cause(Arc<dyn StdError + Send + Sync>);

impl Cause {
    pub fn new(error: impl StdError + Send + Sync + 'static) -> Self {
        Self(Arc::new(error))
    }
}

impl PartialEq for Cause {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for Cause {}

impl fmt::Display for Cause {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl StdError for Cause {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        self.0.source()
    }
}

/// Why a package could not be read, stored or removed.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum PackageError {
    #[error("the package file is missing")]
    NotFound,
    #[error("the package is not a regular file")]
    NotRegular,
    #[error("the package is larger than {MAX_PACKAGE_BYTES} bytes")]
    TooLarge,
    #[error("the package is not valid UTF-8 text")]
    Encoding,
    #[error("the package is not valid TOML: {reason}")]
    Syntax { reason: String },
    #[error("the package is refused: {0}")]
    Manifest(#[from] ManifestError),
    #[error("a plugin with this id is already installed")]
    Duplicate(PluginId),
    #[error("the stored package no longer has the hash that was consented to")]
    HashChanged,
    #[error("the package could not be accessed: {reason}")]
    Unreadable { reason: String },
    #[error("the package could not be stored or removed")]
    Unwritable(#[source] Cause),
    #[error("`{0}` is not the id of a user plugin, so it names no package file")]
    NotUserId(PluginId),
}

/// Reads, stores and removes packages. Implemented by an adapter.
pub trait PluginPackages: Send + Sync {
    /// Reads a package at `location` without following links, hashes the exact bytes, and
    /// parses the manifest. It reads only; nothing is stored.
    fn read(&self, location: &str) -> Result<Package, PackageError>;

    /// Stores the exact bytes of a read package privately and atomically. A package with the
    /// same plugin id is a duplicate unless `replace` says it is an update.
    fn publish(&self, package: &Package, replace: bool) -> Result<(), PackageError>;

    /// Loads an installed package and checks that its bytes still hash to `expected`.
    fn load(&self, plugin: &PluginId, expected: &ContentHash) -> Result<Package, PackageError>;

    /// Removes an installed package. Removing one that is not installed succeeds.
    fn remove(&self, plugin: &PluginId) -> Result<(), PackageError>;
}

/// Why a runtime could not produce a view or effects.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum RuntimeError {
    #[error("the package has no contribution `{0}`")]
    UnknownContribution(LocalId),
    #[error("the package needs API {needs}, newer than this host")]
    Unsupported { needs: ApiVersion },
    #[error("the plugin trapped")]
    Trap,
    #[error("the plugin passed its deadline")]
    Deadline,
    #[error("the view is invalid")]
    View(#[from] ViewError),
}

/// A package ready to be asked for views and event answers.
pub trait PluginInstance: Send + Sync {
    /// The view of a contribution. It cannot produce effects, by signature.
    fn render(&self, local: &LocalId, snapshot: &Snapshot) -> Result<View, RuntimeError>;

    /// The answer to an event: at most eight effects.
    fn on_event(
        &self,
        local: &LocalId,
        event: Event,
        snapshot: &Snapshot,
    ) -> Result<Effects, RuntimeError>;
}

/// Turns a package into an instance. A declarative interpreter and, later, a code runtime.
pub trait PluginRuntime: Send + Sync {
    fn instantiate(&self, package: &Package) -> Result<Box<dyn PluginInstance>, RuntimeError>;
}
