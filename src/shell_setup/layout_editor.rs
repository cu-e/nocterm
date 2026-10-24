//! The editor of the status bar and panels, placed in Appearance.
use std::sync::Arc;

use nocterm_plugin_model::PluginCatalog;
use nocterm_ui::{EmbeddedPageSpec, LayoutHandle, PluginSettingsDirectory, UiReady};

/// The sections Appearance shows below its own settings. The editor reads the same catalog and
/// plugin settings as the Plugins page, and the frozen `workspace.swapped` flag the windows read.
pub(super) fn appearance_sections(
    catalog: &Arc<PluginCatalog>,
    directory: PluginSettingsDirectory,
    layout: &LayoutHandle,
    ui: &UiReady,
    cx: &gpui_kit::App,
) -> Vec<EmbeddedPageSpec> {
    vec![nocterm_layout_ui::layout_section(
        ui.settings().clone(),
        catalog.clone(),
        directory,
        nocterm_workspace::legacy_layout_swapped(layout, cx),
    )]
}
