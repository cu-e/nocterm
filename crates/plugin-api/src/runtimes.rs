//! Picks the runtime of a package by what its manifest declares.

use alloc::{boxed::Box, sync::Arc};

use crate::{DeclarativeRuntime, Package, PluginInstance, PluginRuntime, Runtime, RuntimeError};

/// The runtimes a host composes: the declarative interpreter, always, and a code runtime when
/// the application supplies one. A package whose runtime is not composed is refused with
/// [`RuntimeError::NoRuntime`], so a build without a code runtime never runs a module.
#[derive(Clone)]
pub struct Runtimes {
    declarative: DeclarativeRuntime,
    code: Option<Arc<dyn PluginRuntime>>,
}

impl Runtimes {
    /// Declarative packages only.
    pub fn new() -> Self {
        Self {
            declarative: DeclarativeRuntime,
            code: None,
        }
    }

    /// Adds the runtime of `Runtime::Wasm` packages.
    #[must_use]
    pub fn with_code(mut self, code: Arc<dyn PluginRuntime>) -> Self {
        self.code = Some(code);
        self
    }
}

impl Default for Runtimes {
    fn default() -> Self {
        Self::new()
    }
}

impl PluginRuntime for Runtimes {
    fn instantiate(&self, package: &Package) -> Result<Box<dyn PluginInstance>, RuntimeError> {
        match package.manifest().runtime() {
            Runtime::Declarative => self.declarative.instantiate(package),
            Runtime::Wasm => self
                .code
                .as_ref()
                .ok_or(RuntimeError::NoRuntime(Runtime::Wasm))?
                .instantiate(package),
        }
    }
}
