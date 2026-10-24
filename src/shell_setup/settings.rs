//! Where each system plugin's settings are stored, for the Plugins page.
use nocterm_ui::{PluginSettingsDirectory, SettingsPageSpec};
use std::{rc::Rc, sync::Arc};

use gpui_kit::App;
use nocterm_plugin_model::{CatalogError, PluginCatalog};
use nocterm_store::SettingsHandle;

use crate::settings_pages::GuestPages;

/// The monitor, the plugin catalog and the lazy settings pages that follow the builtin ones,
/// among them the Plugins page over the same catalog.
pub(crate) fn install_guest_pages(
    local_host: &Option<Arc<dyn nocterm_session::HostExec>>,
    ui: &nocterm_ui::UiReady,
    keymap: nocterm_keymap::KeymapHandle,
    files: &nocterm_files::Installed,
    projects: Option<&nocterm_projects::Installed>,
    cx: &mut App,
) -> Result<
    (
        nocterm_monitor_ui::Installed,
        Arc<PluginCatalog>,
        GuestPages,
    ),
    CatalogError,
> {
    let monitor = nocterm_monitor_ui::install(local_host.clone(), ui, cx);
    let plugins = Arc::new(super::plugin_catalog()?);
    let page = plugins_page(&plugins, &monitor, ui.settings(), cx);
    let guests = crate::settings_pages::guest_pages(
        keymap,
        &monitor,
        files,
        projects,
        page,
        plugins.clone(),
    );
    Ok((monitor, plugins, guests))
}

/// The Plugins page over the application catalog, writing `[plugins]` through `handle`.
fn plugins_page(
    catalog: &Arc<PluginCatalog>,
    monitor: &nocterm_monitor_ui::Installed,
    handle: &SettingsHandle,
    cx: &mut App,
) -> SettingsPageSpec {
    nocterm_plugins_ui::settings_page(
        catalog.clone(),
        Rc::new(super::AppPluginToggles::new(
            catalog.clone(),
            handle.clone(),
            cx,
        )),
        plugin_settings(monitor),
    )
}

/// The schema and source of every system plugin that has settings. Each source writes the
/// owner's own section, so a page and the owner's page edit the same values.
fn plugin_settings(monitor: &nocterm_monitor_ui::Installed) -> PluginSettingsDirectory {
    PluginSettingsDirectory::new().with_system(
        &nocterm_monitor_ui::plugin::PLUGIN,
        nocterm_monitor_ui::plugin_settings(monitor),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::TestAppContext;

    #[gpui_kit::test]
    fn every_plugin_with_settings_has_exactly_one_source(cx: &mut TestAppContext) {
        let catalog = crate::shell_setup::plugin_catalog().expect("the application catalog");
        let directory = cx.update(|cx| {
            gpui_kit::init(cx);
            let store = nocterm_store::SettingsConfig::in_memory(Default::default()).install(cx);
            let ready = nocterm_ui::init(nocterm_ui::DesignTokens::builtin(), store, cx);
            plugin_settings(&nocterm_monitor_ui::install(None, &ready, cx))
        });
        for plugin in catalog.plugins() {
            let has_settings = !plugin.settings.groups.is_empty();
            let source = directory.get(&plugin.id);
            assert_eq!(
                source.is_some(),
                has_settings,
                "{} needs a source exactly when it has settings",
                plugin.id
            );
            if let Some((schema, _)) = source {
                assert!(std::ptr::eq(schema, &plugin.settings));
            }
        }
    }
}
