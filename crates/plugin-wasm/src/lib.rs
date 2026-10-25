//! The code runtime of user plugins: a WebAssembly module on wasmi, behind the
//! [`PluginRuntime`](nocterm_plugin_api::PluginRuntime) port.
//!
//! Owns: the wasmi engine configuration, the compiled-module cache, the live-instance budget,
//!       one guest per instance (store, memory and the ABI exports), the mapping of traps to
//!       `RuntimeError` and the bounded guest log.
//! Does not own: package files and hashing (nocterm-plugin-packages), the wire types and the
//!       ABI constants (nocterm-plugin-abi), manifests, views and effects
//!       (nocterm-plugin-api), where log lines go (the application's `GuestLog`).
//! Ports used: `GuestLog`, one method, injected into [`WasmRuntime::new`]. Ports provided:
//!       `PluginRuntime`, `PluginInstance`.
//! Settings: none. Globals: none.
//! Invariants: a guest imports nothing, so it reaches the host only through the postcard reply
//!       of `nocterm_call`; every call burns at most `CALL_FUEL`, every instance holds at most
//!       `MEMORY_BYTES` and at most `MAX_LIVE_INSTANCES` are live; every read of guest memory
//!       is bounds-checked; a guest that failed is dropped and the next call starts it fresh
//!       from the cached module; the crate touches no file and spawns no thread. Calls run on
//!       the caller's thread (the UI thread: a deliberate X-ship gate item, see ADR 0091).
#![forbid(unsafe_code)]

mod budget;
mod cache;
mod engine;
mod guest;
mod instance;
mod log;
mod runtime;
mod trap;

pub use log::GuestLog;
pub use runtime::WasmRuntime;

#[cfg(test)]
mod tests;
