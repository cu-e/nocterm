//! Ports of the user plugin track: where packages come from and how a package's code runs.
//!
//! The api crate only names them. The adapter `nocterm-plugin-packages` implements
//! [`PluginPackages`]; the declarative interpreter in this crate and the code runtime adapter
//! `nocterm-plugin-wasm` implement [`PluginRuntime`], and [`Runtimes`](crate::Runtimes) picks one
//! by the manifest.

use alloc::{boxed::Box, string::String, sync::Arc, vec::Vec};
use core::{error::Error as StdError, fmt};

use nocterm_plugin_model::PluginId;
use thiserror::Error;

use crate::{
    ApiVersion, ContentHash, Effects, Event, LocalId, Manifest, ManifestError, PackageKind,
    Runtime, SectionError, Snapshot, View, ViewError,
};

/// The largest declarative package (a TOML manifest), in bytes.
pub const MAX_PACKAGE_BYTES: u64 = 256 * 1024;
/// The largest code package (a WebAssembly module), in bytes.
pub const MAX_MODULE_BYTES: u64 = 2 * 1024 * 1024;

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
    #[error(
        "the package is larger than its limit ({MAX_PACKAGE_BYTES} bytes of TOML, {MAX_MODULE_BYTES} bytes of code)"
    )]
    TooLarge,
    #[error("the package is not valid UTF-8 text")]
    Encoding,
    #[error("the package is not valid TOML: {reason}")]
    Syntax { reason: String },
    #[error("the module does not carry a readable manifest: {0}")]
    Section(#[from] SectionError),
    #[error("the manifest says `{declared:?}` but the file is {kind:?}")]
    RuntimeMismatch {
        declared: Runtime,
        kind: PackageKind,
    },
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

/// Why a code module cannot be run at all.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum ModuleError {
    #[error("the module is not valid WebAssembly or breaks a limit")]
    Invalid(#[source] Cause),
    #[error("the module imports something; ABI 1 offers no imports")]
    Imports,
    #[error("the module lacks an export of the ABI or gives it the wrong type")]
    Exports,
    #[error("the module speaks ABI {found}")]
    AbiVersion { found: i32 },
}

/// How a code plugin's reply broke the call protocol.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum ProtocolError {
    /// A pointer or a length the guest gave does not lie inside its memory.
    #[error("a buffer lies outside the guest's memory")]
    OutsideMemory,
    #[error("the reply is larger than {max} bytes")]
    ReplyTooLarge { max: usize },
    #[error("the reply is not a value of the ABI")]
    Undecodable(#[source] Cause),
    /// A view where effects were asked for, or the other way round.
    #[error("the answer is not the kind the call asked for")]
    WrongAnswer,
    #[error("the reply carries more than {max} log lines")]
    TooManyLogLines { max: usize },
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
    #[error("no runtime is composed for `{0:?}` packages")]
    NoRuntime(Runtime),
    #[error("the module is refused")]
    Module(#[from] ModuleError),
    #[error("the plugin passed its memory limit")]
    Memory,
    #[error("the plugin broke the call protocol")]
    Malformed(#[from] ProtocolError),
    /// The host could not hand the call to the guest because its snapshot is too large. The
    /// plugin did nothing wrong, so this is not a fault.
    #[error("the call is larger than a guest accepts")]
    InputTooLarge,
    #[error("too many plugins are running")]
    Capacity,
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

/// Turns a package into an instance: the declarative interpreter or a code runtime.
pub trait PluginRuntime: Send + Sync {
    fn instantiate(&self, package: &Package) -> Result<Box<dyn PluginInstance>, RuntimeError>;
}
