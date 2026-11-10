//! Which source holds the settings of which plugin.

use std::{collections::BTreeMap, ops::Deref, rc::Rc, sync::Arc};

use gpui_kit::App;
use nocterm_plugin_model::{PluginId, SettingsSchema, SystemPlugin};

use super::PluginSettingsSource;

/// The schema of a plugin's settings: a system plugin's const, or a user plugin's own.
#[derive(Clone)]
pub enum SchemaRef {
    System(&'static SettingsSchema),
    User(Arc<SettingsSchema>),
}

impl Deref for SchemaRef {
    type Target = SettingsSchema;

    fn deref(&self) -> &SettingsSchema {
        match self {
            Self::System(schema) => schema,
            Self::User(schema) => schema,
        }
    }
}

type Entry = (SchemaRef, Rc<dyn PluginSettingsSource>);

/// Finds the settings of installed user plugins. They come and go while the application runs,
/// so a directory asks for them each time.
pub trait UserPluginSettings: 'static {
    /// The schema and source of `plugin`, if it is installed and declares settings.
    fn find(&self, plugin: &PluginId, cx: &App) -> Option<Entry>;
}

/// The settings schema and source of every plugin that has settings.
///
/// The application composes it once and passes it to the views that show plugin settings.
#[derive(Clone, Default)]
pub struct PluginSettingsDirectory {
    system: BTreeMap<PluginId, Entry>,
    user: Option<Rc<dyn UserPluginSettings>>,
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
        self.system.insert(
            plugin.id.clone(),
            (SchemaRef::System(&plugin.settings), source),
        );
        self
    }

    /// Adds where the settings of user plugins are found. A system plugin id is never looked up
    /// there, and a `user-` id is never looked up among the system plugins.
    pub fn with_user(mut self, user: Rc<dyn UserPluginSettings>) -> Self {
        self.user = Some(user);
        self
    }

    pub fn get(&self, plugin: &PluginId, cx: &App) -> Option<Entry> {
        if plugin.is_user() {
            return self.user.as_ref()?.find(plugin, cx);
        }
        self.system.get(plugin).cloned()
    }
}
