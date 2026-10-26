//! Faults of a plugin's code per plugin in this session.

use std::collections::BTreeMap;

use nocterm_plugin_api::RuntimeError;
use nocterm_plugin_model::PluginId;

use crate::caps::MAX_FAULTS;

/// The fault counts of this session. They are never saved.
#[derive(Default)]
pub(crate) struct Faults {
    counts: BTreeMap<PluginId, u8>,
}

impl Faults {
    /// Whether a runtime error is the plugin's fault: a trap, a missed deadline, passing the
    /// memory limit, breaking the call protocol, or a module the runtime refuses. Being refused
    /// for lack of a runtime or of room, or because the host's own call was too large to hand
    /// over, is the host's situation and does not count. (A view or effects the host refuses to
    /// use are counted where they are checked, for plugins that run code.)
    pub(crate) fn counts(error: &RuntimeError) -> bool {
        matches!(
            error,
            RuntimeError::Trap
                | RuntimeError::Deadline
                | RuntimeError::Memory
                | RuntimeError::Malformed(_)
                | RuntimeError::Module(_)
        )
    }

    /// Counts one fault. True when it was the one that disables the plugin.
    pub(crate) fn record(&mut self, plugin: &PluginId) -> bool {
        let count = self.counts.entry(plugin.clone()).or_default();
        *count = count.saturating_add(1);
        *count == MAX_FAULTS
    }

    pub(crate) fn clear(&mut self, plugin: &PluginId) {
        self.counts.remove(plugin);
    }
}
