//! Traps and missed deadlines per plugin in this session.

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
    /// Whether a runtime error is the plugin's fault: a trap or a missed deadline.
    pub(crate) fn counts(error: &RuntimeError) -> bool {
        matches!(error, RuntimeError::Trap | RuntimeError::Deadline)
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
