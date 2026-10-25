//! The window shell's plugin catalog: the one list of system plugins an installation knows.
use std::{rc::Rc, sync::Arc};

use nocterm_plugin_model::{CatalogError, PluginCatalog, PluginSet, SystemPlugin};
use nocterm_ui::{PluginSource, StaticPluginSource};
use nocterm_workspace::plugin;

mod layout_editor;
mod plugins;
#[cfg(test)]
mod reference;
mod settings;
#[cfg(all(test, feature = "user-plugins"))]
mod user_plugin_contract;
#[cfg(all(test, feature = "user-plugins"))]
mod user_plugins;

pub(crate) use plugins::AppPluginToggles;
pub(crate) use settings::install_guest_pages;

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

/// What the windows read the installed plugins from. The default build has the system plugins
/// only, so the set never changes.
pub(crate) fn plugin_source(
    catalog: &Arc<PluginCatalog>,
    _cx: &mut gpui_kit::App,
) -> Rc<dyn PluginSource> {
    Rc::new(StaticPluginSource::new(PluginSet::system(catalog.clone())))
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
