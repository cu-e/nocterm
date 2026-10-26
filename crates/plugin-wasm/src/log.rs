//! Where the log lines of a guest go, and how many get there.

use std::sync::Arc;

use nocterm_plugin_abi::{MAX_LOG_LINE_CHARS, MAX_LOG_LINES};
use nocterm_plugin_api::sanitize_display;
use nocterm_plugin_model::PluginId;

/// Lines one instance may log in its lifetime; after that one notice replaces the rest.
const MAX_LINES_PER_INSTANCE: usize = 256;
const SUPPRESSED: &str = "further log lines of this plugin are suppressed";

/// The sink for what a guest says. The application decides where it goes (a tracing target,
/// a file); the runtime only bounds and cleans the text.
pub trait GuestLog: Send + Sync {
    /// One cleaned line from `plugin`.
    fn line(&self, plugin: &PluginId, text: &str);
}

/// What one instance has logged so far.
pub(crate) struct LogBudget {
    plugin: PluginId,
    sink: Arc<dyn GuestLog>,
    emitted: usize,
}

impl LogBudget {
    pub(crate) fn new(plugin: PluginId, sink: Arc<dyn GuestLog>) -> Self {
        Self {
            plugin,
            sink,
            emitted: 0,
        }
    }

    /// Passes on at most [`MAX_LOG_LINES`] of `lines`, each sanitized and cut, until the
    /// instance has used its lifetime budget.
    pub(crate) fn record(&mut self, lines: &[String]) {
        for text in lines.iter().take(MAX_LOG_LINES) {
            if self.emitted == MAX_LINES_PER_INSTANCE {
                self.sink.line(&self.plugin, SUPPRESSED);
                self.emitted += 1;
            }
            if self.emitted > MAX_LINES_PER_INSTANCE {
                return;
            }
            self.sink
                .line(&self.plugin, &sanitize_display(text, MAX_LOG_LINE_CHARS));
            self.emitted += 1;
        }
    }
}
