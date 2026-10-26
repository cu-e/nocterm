//! Which system plugins the user switched off.
//!
//! The `[plugins]` settings section lists the plugins the user disabled. A disabled plugin
//! contributes nothing to the window shell: its panels and widgets are not instantiated, while
//! its feature keeps running (actions, services and settings page). [`PluginEnablement`] stores
//! the list and [`PluginEnablement::effective_disabled`] turns it into the set the shell applies
//! against a [`PluginCatalog`]: required plugins and ids
//! the catalog does not know are ignored there, so a hand edit cannot switch off a required
//! plugin. Core contributions are not plugins, so their namespaces are unknown ids here and are
//! never disabled.
//!
//! User plugins are not listed here: their enabled flag lives in the host's records, so a
//! `user-` id in this section is dropped when it is read and never written. The section holds
//! only the one switch that lets a build with the `user-plugins` feature load them at all.
//!
//! Owns: the `[plugins]` section, its sanitizing, the effective disabled set and the
//! `user_plugins` switch.
//! Does not own: the enabled state of a user plugin (the host's records), which plugins exist or
//! are required (`nocterm-plugin-model`), how a disabled
//! plugin is left out of a layout (`nocterm-window-layout`), instantiating panels and widgets
//! (`nocterm-workspace`), the Plugins page, settings storage.
//! Ports used/provided: none; it defines a section for `nocterm-settings`. Settings section:
//! plugins. Globals: none.
//! Invariants: a table that does not read is reported by [`PluginEnablement::read_error`], so no
//! writer replaces it; the stored list holds valid, non-`user-` ids once each and at most [`MAX_DISABLED`] of them;
//! ids missing from this build are kept, so a plugin that returns finds its state; the effective
//! set never holds a required plugin or an id outside the catalog. No I/O.

use std::collections::BTreeSet;

use nocterm_plugin_model::{PluginCatalog, PluginId};
use nocterm_settings::{SettingsDocument, SettingsSection};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// What a settings document says about loading user plugins at the next start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UserPluginsChoice {
    /// The user allowed them.
    Allowed,
    /// Absent or switched off.
    Off,
    /// The `[plugins]` table does not read, so the switch in it is not taken as allowed.
    Unreadable {
        /// Why the table does not read.
        reason: String,
    },
}

/// Most plugin ids the section keeps.
pub const MAX_DISABLED: usize = 64;

/// The plugins the user disabled.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct PluginEnablement {
    /// Ids of the disabled system plugins. A disabled plugin's panels and widgets are not shown
    /// in any window; its settings stay available. Required plugins cannot be disabled.
    #[schemars(with = "Vec<String>")]
    pub disabled: Vec<PluginId>,
    /// Whether a build with the `user-plugins` feature loads user plugins at the start. Off by
    /// default and read only by such builds, once, at the start: a change applies at the next
    /// start. Release builds are made without the feature, so the switch does nothing there.
    /// Left out of the file while off.
    #[serde(skip_serializing_if = "core::ops::Not::not")]
    #[schemars(extend("default" = false))]
    pub user_plugins: bool,
}

impl SettingsSection for PluginEnablement {
    const KEY: &'static str = "plugins";

    fn sanitize(&mut self) {
        let mut seen = BTreeSet::new();
        self.disabled
            .retain(|id| id.check().is_ok() && !id.is_user() && seen.insert(id.clone()));
        self.disabled.truncate(MAX_DISABLED);
    }
}

impl PluginEnablement {
    /// Why the document's `[plugins]` table could not be read, if it could not.
    ///
    /// Such a table runs on the defaults, so every plugin looks enabled, and it stays as
    /// written until the section is changed. Changing it, even to switch one plugin, would
    /// replace what the user wrote, so a caller refuses to write while this is `Some`.
    pub fn read_error(document: &SettingsDocument) -> Option<String> {
        document
            .errors()
            .into_iter()
            .find(|error| error.key == Self::KEY)
            .map(|error| error.error)
    }

    /// The plugins the shell leaves out: the stored ones that the catalog knows and does not
    /// require.
    pub fn effective_disabled(&self, catalog: &PluginCatalog) -> BTreeSet<PluginId> {
        self.disabled
            .iter()
            .filter(|id| catalog.plugin(id).is_some_and(|plugin| !plugin.required))
            .cloned()
            .collect()
    }

    /// Whether the user allowed user plugins to load at the start.
    pub fn allows_user_plugins(&self) -> bool {
        self.user_plugins
    }

    /// What `document` says about user plugins. A `[plugins]` table that does not read is never
    /// taken as allowed, whatever it holds for the switch.
    pub fn user_plugins_in(document: &SettingsDocument) -> UserPluginsChoice {
        match Self::read_error(document) {
            Some(reason) => UserPluginsChoice::Unreadable { reason },
            None if document.get::<Self>().allows_user_plugins() => UserPluginsChoice::Allowed,
            None => UserPluginsChoice::Off,
        }
    }

    /// Stores whether user plugins may load at the next start.
    pub fn set_user_plugins(&mut self, allowed: bool) {
        self.user_plugins = allowed;
    }

    /// Switches a plugin on or off in the stored list.
    ///
    /// A user plugin id is ignored: user plugins are not stored in this section. When the list
    /// holds [`MAX_DISABLED`] ids already, the ones `catalog` does not know are dropped first (they
    /// have no effect in this build), so the id being switched off is never lost to the cap.
    pub fn set_enabled(&mut self, catalog: &PluginCatalog, id: &PluginId, enabled: bool) {
        if id.is_user() {
            return;
        }
        if enabled {
            self.disabled.retain(|listed| listed != id);
        } else if !self.disabled.contains(id) {
            if self.disabled.len() >= MAX_DISABLED {
                self.disabled
                    .retain(|listed| catalog.plugin(listed).is_some());
            }
            self.disabled.push(id.clone());
        }
    }
}

#[cfg(test)]
mod tests;
