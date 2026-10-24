//! The window shell's plugin catalog: the one list of system plugins an installation knows.
use nocterm_plugin_model::{CatalogError, PluginCatalog, SystemPlugin};
use nocterm_workspace::plugin;

mod plugins;
#[cfg(test)]
mod reference;
mod settings;

pub(crate) use plugins::AppPluginToggles;
pub(crate) use settings::install_guest_pages;

/// Every system plugin of the application. A plugin appears here once, beside the others,
/// so the catalog check sees all contributions together.
const PLUGINS: &[&SystemPlugin] = &[
    &nocterm_snippets_ui::plugin::PLUGIN,
    &nocterm_containers_ui::plugin::PLUGIN,
    &nocterm_projects::plugin::PLUGIN,
    &nocterm_monitor_ui::plugin::PLUGIN,
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
