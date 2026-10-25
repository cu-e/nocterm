//! The window shell's plugin catalog: the one list of system plugins an installation knows.
use std::{rc::Rc, sync::Arc};

use gpui_kit::App;
use nocterm_core::{Paths, clock::WallClock};
use nocterm_plugin_model::{CatalogError, PluginCatalog, SystemPlugin};
use nocterm_store::ShutdownCoordinator;
use nocterm_ui::PluginSource;
use nocterm_workspace::plugin;

mod layout_editor;
mod plugins;
#[cfg(test)]
mod reference;
mod settings;
#[cfg(all(test, feature = "user-plugins"))]
mod user_plugin_contract;
#[cfg(feature = "user-plugins")]
mod user_plugin_panel;
// The default build has no user plugins and composes nothing in their place.
#[cfg(feature = "user-plugins")]
mod user_plugins;
#[cfg(not(feature = "user-plugins"))]
#[path = "shell_setup/no_user_plugins.rs"]
mod user_plugins;

pub(crate) use plugins::AppPluginToggles;
pub(crate) use settings::{GuestInputs, install_guest_pages};
pub(crate) use user_plugins::UserShell;
#[cfg(feature = "user-plugins")]
pub(crate) use user_plugins::check_directory;

/// Every system plugin of the application. A plugin appears here once, beside the others,
/// so the catalog check sees all contributions together.
const PLUGINS: &[&SystemPlugin] = &[
    &nocterm_snippets_ui::plugin::PLUGIN,
    &nocterm_containers_ui::plugin::PLUGIN,
    &nocterm_projects::plugin::PLUGIN,
    &nocterm_monitor_ui::plugin::PLUGIN,
    &nocterm_workspace::clock::PLUGIN,
    &nocterm_agent::plugin::PLUGIN,
];

/// Checks the shell's groups and contributions and the system plugins as one catalog.
pub(crate) fn plugin_catalog() -> Result<PluginCatalog, CatalogError> {
    plugin::catalog(
        &[
            nocterm_connections::plugin::CORE,
            nocterm_files::plugin::CORE,
        ],
        PLUGINS,
    )
}

/// What the user plugins of this run are: the installed ones, or the one file of a developer
/// session (`nocterm plugin dev`). X-ship adds a safe mode here, not another field on the
/// services.
#[derive(Clone, Debug, Default)]
pub(crate) enum UserPluginMode {
    #[default]
    Installed,
    /// Load this file for consent in a session that loads no installed plugin, keeps the
    /// plugins' records, data and packages in memory, and does not write `settings.toml`.
    #[cfg(feature = "user-plugins")]
    Dev(std::path::PathBuf),
}

impl UserPluginMode {
    /// Whether the run writes the user's settings file. A developer session reads it but keeps
    /// every change in memory: the plugin set it publishes lacks the installed plugins, and the
    /// layout editor would move their placed entries to the tray and write that back.
    pub(crate) fn saves_settings(&self) -> bool {
        match self {
            Self::Installed => true,
            #[cfg(feature = "user-plugins")]
            Self::Dev(_) => false,
        }
    }
}

/// The plugins of the installation: the system catalog, what the windows read the installed
/// plugins from, and the user plugins behind the `user-plugins` feature.
#[derive(Clone)]
pub(crate) struct Plugins {
    catalog: Arc<PluginCatalog>,
    source: Rc<dyn PluginSource>,
    user: UserShell,
}

impl Plugins {
    /// Checks the catalog and opens the user plugins of `mode`: those stored under `paths`, or
    /// the developer session's.
    pub(crate) fn install(
        paths: &Paths,
        shutdown: &ShutdownCoordinator,
        clock: Arc<dyn WallClock>,
        mode: UserPluginMode,
        cx: &mut App,
    ) -> Result<Self, CatalogError> {
        let catalog = Arc::new(plugin_catalog()?);
        let user = UserShell::install(&catalog, paths, shutdown, clock, mode, cx);
        let source = user.source(&catalog);
        Ok(Self {
            catalog,
            source,
            user,
        })
    }

    pub(crate) fn catalog(&self) -> &Arc<PluginCatalog> {
        &self.catalog
    }

    /// What the windows and the editors read the current plugin set from.
    pub(crate) fn source(&self) -> Rc<dyn PluginSource> {
        self.source.clone()
    }

    pub(crate) fn user(&self) -> &UserShell {
        &self.user
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_application_catalog_lists_the_shell_and_its_groups() {
        let catalog = plugin_catalog().expect("the application catalog is valid");

        assert!(
            catalog
                .contribution(&plugin::NOTIFICATIONS.id)
                .is_some_and(|contribution| contribution.pinned)
        );
        assert_eq!(catalog.groups().len(), plugin::GROUPS.len());
    }
}
