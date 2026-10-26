//! The settings of user plugins: the schema of the installed package and the values the host
//! keeps with the plugin's data. They never reach `[plugins]` or any other settings section.
use std::{rc::Rc, sync::Arc};

use gpui_kit::{App, Entity, Subscription, Task};
use nocterm_core::SharedCause;
use nocterm_plugin_host::PluginHost;
use nocterm_plugin_model::{FieldEdit, PluginId, SettingValues, SettingsSchema, apply};
use nocterm_ui::{PluginSettingsError, PluginSettingsSource, SchemaRef, UserPluginSettings};

/// Finds the settings of the plugins the host runs.
pub(super) struct HostSettings {
    host: Entity<PluginHost>,
}

impl HostSettings {
    pub(super) fn new(host: Entity<PluginHost>) -> Self {
        Self { host }
    }
}

impl UserPluginSettings for HostSettings {
    fn find(
        &self,
        plugin: &PluginId,
        cx: &App,
    ) -> Option<(SchemaRef, Rc<dyn PluginSettingsSource>)> {
        let schema = self.host.read(cx).manifest(plugin)?.plugin().settings();
        if schema.is_empty() {
            return None;
        }
        let schema = Arc::new(schema.clone());
        let values = HostValues {
            host: self.host.clone(),
            plugin: plugin.clone(),
            schema: schema.clone(),
        };
        Some((SchemaRef::User(schema), Rc::new(values)))
    }
}

/// One plugin's values, stored by the host.
struct HostValues {
    host: Entity<PluginHost>,
    plugin: PluginId,
    schema: Arc<SettingsSchema>,
}

impl PluginSettingsSource for HostValues {
    fn values(&self, cx: &App) -> SettingValues {
        self.host
            .read(cx)
            .values(&self.plugin)
            .unwrap_or_else(|| self.schema.defaults())
    }

    fn write(
        &self,
        key: &str,
        edit: FieldEdit,
        cx: &mut App,
    ) -> Task<Result<(), PluginSettingsError>> {
        let mut next = self.values(cx);
        let value = match apply(&self.schema, &next, key, edit) {
            Ok(value) => value,
            Err(error) => return Task::ready(Err(PluginSettingsError::Field(error))),
        };
        next.insert(key, value);
        let stored = self
            .host
            .update(cx, |host, cx| host.replace_values(&self.plugin, &next, cx));
        Task::ready(stored.map_err(|error| PluginSettingsError::Write(SharedCause::new(error))))
    }

    fn observe(&self, cx: &mut App, mut on_change: Box<dyn FnMut(&mut App)>) -> Subscription {
        cx.observe(&self.host, move |_, cx| on_change(cx))
    }
}
