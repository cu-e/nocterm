//! The runtimes of user plugins and where their guests log.
use std::sync::Arc;

use nocterm_plugin_api::Runtimes;
use nocterm_plugin_model::PluginId;
use nocterm_plugin_wasm::{GuestLog, WasmRuntime};

/// The declarative interpreter plus the code runtime, whose guests log to `log`.
pub(super) fn runtimes(log: Arc<dyn GuestLog>) -> Arc<Runtimes> {
    Arc::new(Runtimes::new().with_code(Arc::new(WasmRuntime::new(log))))
}

/// Guest log lines of installed plugins go to the application log at `debug`, so a plugin
/// cannot flood production logs.
pub(super) struct TracingGuestLog;

impl GuestLog for TracingGuestLog {
    fn line(&self, plugin: &PluginId, text: &str) {
        tracing::debug!(target: "nocterm::plugin", plugin = %plugin, "{text}");
    }
}

/// `nocterm plugin check` does not initialise tracing, so guest lines have nowhere to go.
pub(super) struct DiscardGuestLog;

impl GuestLog for DiscardGuestLog {
    fn line(&self, _plugin: &PluginId, _text: &str) {}
}
