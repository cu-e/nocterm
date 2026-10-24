//! The Plugins settings page: every system plugin with a switch that turns it on or off, the
//! settings form its schema declares, and the way to the plugin's own Settings page.
//!
//! Owns: the page layout and selection, the enable switch with its refusal text, the plugin
//! settings form per selected plugin, and revealing a plugin by id.
//! Does not own: what disabling does (the workspace), where the choice or the values are stored,
//! the settings host and its navigation, or the plugins' own pages.
//! Ports used: `PluginToggles` (implemented by the application over `[plugins]`) and
//! `PluginSettingsDirectory` (the source of each plugin's values). Provided: a lazy
//! `SettingsPageSpec` with id `plugins`, built by [`settings_page`].
//! Settings section: none. Globals: none.
//! Invariants: the list shows plugins, never core contributions; a plugin that is required has
//! a switch that cannot be turned off; the form shows only what its source holds, and text typed
//! into it is sent when the selection changes or the page is hidden or closed.

mod page;
mod toggles;
mod view;

use std::{rc::Rc, sync::Arc};

use gpui_kit::AppContext as _;
use nocterm_plugin_model::PluginCatalog;
use nocterm_ui::{IconName, PluginSettingsDirectory, SettingsPageSpec};

pub use page::PluginsPage;
pub use toggles::{EnablementError, PluginToggles};

/// The lazy Plugins page for the application Settings host.
pub fn settings_page(
    catalog: Arc<PluginCatalog>,
    toggles: Rc<dyn PluginToggles>,
    directory: PluginSettingsDirectory,
) -> SettingsPageSpec {
    SettingsPageSpec::new("plugins", "Plugins", move |window, cx| {
        cx.new(|cx| {
            PluginsPage::new(
                catalog.clone(),
                toggles.clone(),
                directory.clone(),
                window,
                cx,
            )
        })
    })
    .with_icon(IconName::Plug)
}

#[cfg(test)]
mod tests;
