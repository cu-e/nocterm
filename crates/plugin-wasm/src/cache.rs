//! Compiled modules by package hash, so a restart or a second window does not compile again.

use std::{
    collections::VecDeque,
    sync::{Mutex, PoisonError},
};

use nocterm_plugin_api::{Cause, ContentHash, ModuleError, Package, RuntimeError};
use wasmi::{Engine, Module};

/// Compiled modules kept at once; the oldest is evicted first.
const MAX_MODULES: usize = 16;

#[derive(Default)]
pub(crate) struct ModuleCache {
    entries: Mutex<VecDeque<(ContentHash, Module)>>,
}

impl ModuleCache {
    /// The compiled module of `package`: from the cache, or compiled now. A module wasmi
    /// refuses (malformed, or past a limit) is `ModuleError::Invalid` and is not cached.
    pub(crate) fn get_or_compile(
        &self,
        engine: &Engine,
        package: &Package,
    ) -> Result<Module, RuntimeError> {
        if let Some(module) = self.find(package.hash()) {
            return Ok(module);
        }
        // Compiled outside the lock: a slow module must not block other windows.
        let module = Module::new(engine, package.bytes())
            .map_err(|error| ModuleError::Invalid(Cause::new(error)))?;
        let mut entries = self.lock();
        if let Some((_, existing)) = entries.iter().find(|(hash, _)| hash == package.hash()) {
            return Ok(existing.clone());
        }
        if entries.len() == MAX_MODULES {
            entries.pop_front();
        }
        entries.push_back((*package.hash(), module.clone()));
        Ok(module)
    }

    fn find(&self, hash: &ContentHash) -> Option<Module> {
        self.lock()
            .iter()
            .find(|(cached, _)| cached == hash)
            .map(|(_, module)| module.clone())
    }

    /// The queue holds plain data, so a panic elsewhere cannot leave it half-updated.
    fn lock(&self) -> std::sync::MutexGuard<'_, VecDeque<(ContentHash, Module)>> {
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.lock().len()
    }
}
