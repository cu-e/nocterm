//! The port the settings form reads and writes through.

use gpui_kit::{App, Subscription, Task};
use nocterm_core::SharedCause;
use nocterm_plugin_model::{FieldEdit, FieldError, SettingValues};

/// Where the values of one plugin live. Named for the form's need.
pub trait PluginSettingsSource: 'static {
    /// The values now.
    fn values(&self, cx: &App) -> SettingValues;

    /// Applies `edit` to field `key` on the value current at write time.
    ///
    /// The edit is admitted before this returns; the task reports whether it was accepted and
    /// stored. Dropping the task does not cancel the edit.
    fn write(
        &self,
        key: &str,
        edit: FieldEdit,
        cx: &mut App,
    ) -> Task<Result<(), PluginSettingsError>>;

    /// Calls `on_change` after a value changed, whoever changed it.
    fn observe(&self, cx: &mut App, on_change: Box<dyn FnMut(&mut App)>) -> Subscription;
}

/// A write the source did not accept. Its text can be shown under the field.
#[derive(Clone, Debug, thiserror::Error)]
pub enum PluginSettingsError {
    /// The schema refused the edit.
    #[error(transparent)]
    Field(#[from] FieldError),
    /// The value was accepted but could not be stored.
    #[error("Could not save the setting.")]
    Write(#[source] SharedCause),
}
