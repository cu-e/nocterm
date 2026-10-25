//! The entry point: turns a code package into an instance.

use std::sync::Arc;

use nocterm_plugin_api::{Package, PluginInstance, PluginRuntime, RuntimeError};
use wasmi::Engine;

use crate::{
    budget::Budget, cache::ModuleCache, engine::engine, instance::WasmInstance, log::GuestLog,
};

/// Runs `runtime = "wasm"` packages on wasmi. One runtime serves every window: modules are
/// compiled once per package hash and at most `MAX_LIVE_INSTANCES` guests are alive.
pub struct WasmRuntime {
    engine: Engine,
    modules: ModuleCache,
    budget: Budget,
    log: Arc<dyn GuestLog>,
}

impl WasmRuntime {
    /// A runtime whose guests log to `log`.
    pub fn new(log: Arc<dyn GuestLog>) -> Self {
        Self {
            engine: engine(),
            modules: ModuleCache::default(),
            budget: Budget::default(),
            log,
        }
    }
}

impl PluginRuntime for WasmRuntime {
    /// Claims a place, compiles the module (or finds it by hash) and starts a guest. The
    /// module's ABI is checked here, so a refusal is reported before any view is asked for.
    fn instantiate(&self, package: &Package) -> Result<Box<dyn PluginInstance>, RuntimeError> {
        let slot = self.budget.claim()?;
        let module = self.modules.get_or_compile(&self.engine, package)?;
        let instance = WasmInstance::start(
            self.engine.clone(),
            module,
            package.manifest().clone(),
            Arc::clone(&self.log),
            slot,
        )?;
        Ok(Box::new(instance))
    }
}
