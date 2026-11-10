//! The port the Plugins page switches plugins on and off through.

use gpui_kit::{App, Subscription, Task};
use nocterm_core::SharedCause;
use nocterm_plugin_model::PluginId;

/// Which plugins are enabled and how to change that. Named for the page's need; the
/// application implements it over the `[plugins]` settings section.
pub trait PluginToggles: 'static {
    /// Whether the plugin is enabled now. A plugin that cannot be switched off always is.
    fn enabled(&self, plugin: &PluginId, cx: &App) -> bool;

    /// Switches the plugin on or off. The change is admitted before this returns; the task
    /// reports whether it was accepted and stored.
    fn set_enabled(
        &self,
        plugin: &PluginId,
        enabled: bool,
        cx: &mut App,
    ) -> Task<Result<(), EnablementError>>;

    /// Calls `on_change` after the enabled set changed, whoever changed it.
    fn observe(&self, cx: &mut App, on_change: Box<dyn FnMut(&mut App)>) -> Subscription;
}

/// A switch the owner refused. Its text can be shown beside the switch.
#[derive(Clone, Debug, thiserror::Error)]
pub enum EnablementError {
    /// The plugin is required and cannot be switched off.
    #[error("{0} is required and cannot be switched off")]
    Required(PluginId),
    /// No plugin has this id.
    #[error("{0} is not a known plugin")]
    Unknown(PluginId),
    /// The `[plugins]` table of the settings file does not read. Switching a plugin would write
    /// the section again and replace the table, so the file is left as the user wrote it.
    #[error(
        "The [plugins] table of settings.toml does not read ({reason}); fix it to switch plugins."
    )]
    Malformed {
        /// What was wrong with the table.
        reason: String,
    },
    /// The choice was accepted but could not be stored. The cause is the error's source.
    #[error("Could not save the setting.")]
    Write(#[source] SharedCause),
}
