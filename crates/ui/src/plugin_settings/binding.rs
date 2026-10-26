//! The system source: a schema over one of the owner's own settings sections.

use gpui_kit::{App, Subscription, Task};
use nocterm_core::SharedCause;
use nocterm_plugin_model::{
    FieldEdit, FieldError, SettingValue, SettingValues, SettingsSchema, apply, bounded,
};
use nocterm_settings::SettingsSection;
use nocterm_store::{SettingsHandle, StoreError};

use super::{PluginSettingsError, PluginSettingsSource};

/// A [`PluginSettingsSource`] over section `S` of `settings.toml`.
///
/// The field keys of the schema are the serde keys of `S`, so no value moves. Every write runs
/// inside [`SettingsHandle::update_checked`]: the edit is applied to the section as it is at
/// that moment, so two quick edits both land, and a refused edit leaves the section untouched.
/// Values read from the section are sanitized and cut to their field's length, so a hand-edited
/// document cannot show hidden or unbounded text.
pub struct SectionBinding<S: SettingsSection> {
    handle: SettingsHandle,
    schema: &'static SettingsSchema,
    read: fn(&S) -> SettingValues,
    write: fn(&mut S, &str, SettingValue) -> Result<(), FieldError>,
}

impl<S: SettingsSection> SectionBinding<S> {
    /// `read` lists the section's values by field key; `write` stores one already validated
    /// value under its key.
    pub fn new(
        handle: SettingsHandle,
        schema: &'static SettingsSchema,
        read: fn(&S) -> SettingValues,
        write: fn(&mut S, &str, SettingValue) -> Result<(), FieldError>,
    ) -> Self {
        Self {
            handle,
            schema,
            read,
            write,
        }
    }
}

impl<S: SettingsSection> PluginSettingsSource for SectionBinding<S> {
    fn values(&self, cx: &App) -> SettingValues {
        bounded(self.schema, (self.read)(self.handle.setting::<S>(cx)))
    }

    fn write(
        &self,
        key: &str,
        edit: FieldEdit,
        cx: &mut App,
    ) -> Task<Result<(), PluginSettingsError>> {
        let (schema, read, write, key) = (self.schema, self.read, self.write, key.to_string());
        let stored = self.handle.update_checked::<S>(cx, move |section| {
            let value = apply(schema, &read(section), &key, edit).map_err(StoreError::edit)?;
            write(section, &key, value).map_err(StoreError::edit)
        });
        cx.spawn(async move |_| stored.await.map(drop).map_err(refusal))
    }

    fn observe(&self, cx: &mut App, on_change: Box<dyn FnMut(&mut App)>) -> Subscription {
        self.handle.observe(cx, on_change)
    }
}

/// A schema refusal comes back as itself; anything else is a failed write.
fn refusal(error: StoreError) -> PluginSettingsError {
    match error {
        StoreError::Edit(cause) => match cause.downcast_ref::<FieldError>() {
            Some(field) => PluginSettingsError::Field(field.clone()),
            None => PluginSettingsError::Write(cause),
        },
        other => PluginSettingsError::Write(SharedCause::new(other)),
    }
}
