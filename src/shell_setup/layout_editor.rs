//! The editor of the status bar and panels, placed in Appearance.
use std::rc::Rc;

use nocterm_ui::{EmbeddedPageSpec, LayoutHandle, PluginSettingsDirectory, PluginSource, UiReady};

/// The sections Appearance shows below its own settings. The editor reads the same catalog and
/// plugin settings as the Plugins page, and the frozen `workspace.swapped` flag the windows read.
pub(super) fn appearance_sections(
    source: &Rc<dyn PluginSource>,
    directory: PluginSettingsDirectory,
    layout: &LayoutHandle,
    ui: &UiReady,
    cx: &gpui_kit::App,
) -> Vec<EmbeddedPageSpec> {
    vec![nocterm_layout_ui::layout_section(
        ui.settings().clone(),
        source.clone(),
        directory,
        nocterm_workspace::legacy_layout_swapped(layout, cx),
    )]
}
