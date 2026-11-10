//! The wasmi engine and the limits of one store.

use nocterm_plugin_abi::MEMORY_BYTES;
use wasmi::{Config, EnforcedLimits, Engine, StoreLimits, StoreLimitsBuilder};

/// Nested calls a guest may make before it traps.
const MAX_RECURSION_DEPTH: usize = 1024;
/// Elements of the one table a module may have.
const MAX_TABLE_ELEMENTS: usize = 10_000;

/// An engine that meters fuel, allows one memory and enforces wasmi's strict module limits. SIMD
/// and 64-bit memories are compiled out by the wasmi features. Function bodies are validated at
/// compile time and translated on first use.
pub(crate) fn engine() -> Engine {
    let mut config = Config::default();
    config
        .consume_fuel(true)
        .wasm_multi_memory(false)
        .ignore_custom_sections(true)
        .set_max_recursion_depth(MAX_RECURSION_DEPTH)
        .enforced_limits(EnforcedLimits::strict());
    Engine::new(&config)
}

/// The limits of one guest: one instance, one memory of at most [`MEMORY_BYTES`], one table.
/// A refused growth traps instead of returning `-1`, so a guest cannot loop on a failed grow.
pub(crate) fn store_limits() -> StoreLimits {
    StoreLimitsBuilder::new()
        .memory_size(MEMORY_BYTES)
        .memories(1)
        .tables(1)
        .table_elements(MAX_TABLE_ELEMENTS)
        .instances(1)
        .trap_on_grow_failure(true)
        .build()
}
