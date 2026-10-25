//! Where each system plugin's settings are stored, for the Plugins page.
use nocterm_ui::{PluginSettingsDirectory, SettingsPageSpec};
use std::rc::Rc;

use gpui_kit::App;
use nocterm_store::SettingsHandle;

use super::Plugins;
use crate::settings_pages::GuestPages;

/// What the guest pages are built from: the installations of the features that own a page.
pub(crate) struct GuestInputs<'a> {
    monitor: &'a nocterm_monitor_ui::Installed,
    ui: &'a nocterm_ui::UiReady,
    keymap: nocterm_keymap::KeymapHandle,
    files: &'a nocterm_files::Installed,
    projects: Option<&'a nocterm_projects::Installed>,
    layout: &'a nocterm_ui::LayoutHandle,
}

impl<'a> GuestInputs<'a> {
    pub(crate) fn new(
        monitor: &'a nocterm_monitor_ui::Installed,
        ui: &'a nocterm_ui::UiReady,
        keymap: nocterm_keymap::KeymapHandle,
        files: &'a nocterm_files::Installed,
        projects: Option<&'a nocterm_projects::Installed>,
        layout: &'a nocterm_ui::LayoutHandle,
    ) -> Self {
        Self {
            monitor,
            ui,
            keymap,
            files,
            projects,
            layout,
        }
    }
}

/// The lazy settings pages that follow the builtin ones, among them the Plugins page over the
/// application catalog, and the editor Appearance shows.
pub(crate) fn install_guest_pages(
    inputs: GuestInputs<'_>,
    plugins: &Plugins,
    cx: &mut App,
) -> GuestPages {
    let GuestInputs {
        monitor,
        ui,
        keymap,
        files,
        projects,
        layout,
    } = inputs;
    let directory = plugins
        .user()
        .directory(plugin_settings(monitor, ui.settings()));
    let page = plugins_page(plugins, directory.clone(), ui.settings(), cx);
    let facts = plugins.user().facts();
    let sections = super::layout_editor::appearance_sections(
        &plugins.source(),
        directory,
        facts,
        layout,
        ui,
        cx,
    );
    crate::settings_pages::guest_pages(
        keymap,
        monitor,
        files,
        projects,
        page,
        plugins.catalog().clone(),
    )
    .with_appearance_sections(sections)
}

/// The Plugins page over the application catalog, writing `[plugins]` through `handle`. The
/// settings of the plugins come from `settings`, which the status bar editor shares.
fn plugins_page(
    plugins: &Plugins,
    settings: PluginSettingsDirectory,
    handle: &SettingsHandle,
    cx: &mut App,
) -> SettingsPageSpec {
    let toggles = Rc::new(super::AppPluginToggles::new(
        plugins.catalog().clone(),
        handle.clone(),
        cx,
    ));
    plugins
        .user()
        .plugins_page(plugins.catalog(), toggles, settings)
}

/// The schema and source of every system plugin that has settings. Each source writes the
/// owner's own section, so a page and the owner's page edit the same values.
fn plugin_settings(
    monitor: &nocterm_monitor_ui::Installed,
    settings: &SettingsHandle,
) -> PluginSettingsDirectory {
    PluginSettingsDirectory::new()
        .with_system(
            &nocterm_monitor_ui::plugin::PLUGIN,
            nocterm_monitor_ui::plugin_settings(monitor),
        )
        .with_system(
            &nocterm_workspace::clock::PLUGIN,
            nocterm_workspace::clock::plugin_settings(settings.clone()),
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
            plugin_settings(
                &nocterm_monitor_ui::install(None, &ready, cx),
                ready.settings(),
            )
        });
        for plugin in catalog.plugins() {
            let has_settings = !plugin.settings.groups.is_empty();
            let source = cx.update(|cx| directory.get(&plugin.id, cx));
            assert_eq!(
                source.is_some(),
                has_settings,
                "{} needs a source exactly when it has settings",
                plugin.id
            );
            if let Some((schema, _)) = source {
                assert!(std::ptr::eq(&*schema, &plugin.settings));
            }
        }
    }
}
