//! The Plugins settings page: every system plugin with a switch that turns it on or off, the
//! settings form its schema declares, and the way to the plugin's own Settings page.
//!
//! Owns: the page layout and selection, the enable switch with its refusal text, the plugin
//! settings form per selected plugin, and revealing a plugin by id.
//! With the cargo feature `user` it also owns the User section: the switch that allows user
//! plugins at the next start and the text that says why a run holds them, the user plugin list,
//! the install and update consent (inert for 750 ms, Deny by default), enabling, placement,
//! removal and the audit log, all through the `UserPlugins` and `UserPluginsSwitch` ports that
//! the application implements. A held run (safe mode, or not allowed) shows no list.
//! Does not own: what disabling does (the workspace), where the choice or the values are stored,
//! the settings host and its navigation, or the plugins' own pages.
//! Ports used: `PluginToggles` (implemented by the application over `[plugins]`) and
//! `PluginSettingsDirectory` (the source of each plugin's values), and with `user` also
//! `UserPlugins` and `UserPluginsSwitch` (the stored "allow user plugins" choice, which applies
//! at the next start). Provided: a lazy `SettingsPageSpec` with id `plugins`, built by
//! [`settings_page`].
//! Settings section: none. Globals: none.
//! Invariants: the list shows plugins, never core contributions; a plugin that is required has
//! a switch that cannot be turned off; the form shows only what its source holds, and text typed
//! into it is sent when the selection changes or the page is hidden or closed.

mod page;
mod toggles;
#[cfg(feature = "user")]
mod user;
mod view;

use std::{rc::Rc, sync::Arc};

use gpui_kit::AppContext as _;
use nocterm_plugin_model::PluginCatalog;
use nocterm_ui::{IconName, PluginSettingsDirectory, SettingsPageSpec};

pub use page::PluginsPage;
pub use toggles::{EnablementError, PluginToggles};
#[cfg(feature = "user")]
pub use user::{
    AuditLine, Candidate, CapabilityLine, FieldLine, HeldBy, InstallRequest, Replaced, RowState,
    UserPluginError, UserPluginRow, UserPlugins, UserPluginsSection, UserPluginsSwitch,
    UserSession,
};

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

/// The lazy Plugins page with a User section below the system plugins, which stay unchanged.
#[cfg(feature = "user")]
pub fn settings_page_with_user_plugins(
    catalog: Arc<PluginCatalog>,
    toggles: Rc<dyn PluginToggles>,
    directory: PluginSettingsDirectory,
    session: UserSession,
    switch: Rc<dyn UserPluginsSwitch>,
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
            .with_user_plugins(session.clone(), switch.clone(), cx)
        })
    })
    .with_icon(IconName::Plug)
}

#[cfg(test)]
mod tests;
