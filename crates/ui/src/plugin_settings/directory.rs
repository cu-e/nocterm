//! Which source holds the settings of which plugin.

use std::{collections::BTreeMap, rc::Rc};

use nocterm_plugin_model::{PluginId, SettingsSchema, SystemPlugin};

use super::PluginSettingsSource;

type Entry = (&'static SettingsSchema, Rc<dyn PluginSettingsSource>);

/// The settings schema and source of every system plugin that has settings.
///
/// The application composes it once and passes it to the views that show plugin settings.
#[derive(Clone, Default)]
pub struct PluginSettingsDirectory {
    system: BTreeMap<PluginId, Entry>,
}

impl PluginSettingsDirectory {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds the settings of `plugin`, stored by `source`.
    pub fn with_system(
        mut self,
        plugin: &'static SystemPlugin,
        source: Rc<dyn PluginSettingsSource>,
    ) -> Self {
        self.system
            .insert(plugin.id.clone(), (&plugin.settings, source));
        self
    }

    pub fn get(&self, plugin: &PluginId) -> Option<Entry> {
        self.system.get(plugin).cloned()
    }
}
