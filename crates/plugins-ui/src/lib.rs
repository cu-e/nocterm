//! The Plugins settings page: every plugin in one list, the system plugins (Built-in) and, with
//! user plugins, the installed ones (External), each with a switch that turns it on or off, the
//! settings form its schema declares, and the way to the plugin's own Settings page. One plugin
//! is selected at a time, of either kind.
//!
//! Owns: the page layout and selection, the enable switch with its refusal text, the plugin
//! settings form per selected plugin, and revealing a plugin by id.
//! It also owns the User section: the switch that allows user
//! plugins at the next start and the text that says why a run holds them, the user plugin list,
//! the install and update consent (inert for 750 ms, Deny by default), enabling, placement,
//! removal (one small button that asks in a dialog what goes) and the audit log, with long
//! lists of the detail folded behind **Show all**, all through the `UserPlugins` and
//! `UserPluginsSwitch` ports that
//! the application implements, and the install row: **Install plugin…**, **Open plugins
//! folder** and the plugin files found there, through the `PluginsFolder` port. The page draws
//! the section in parts (`UserPart`): its rows in the plugin list, the selected plugin's detail
//! beside it, and the switch, install, folder and consent below. One switch turns a user plugin
//! on (enabled and allowed in windows) or off (disabled, its place kept). A held run (safe
//! mode, or not allowed) lists none; its install row opens the folder and says what to do
//! first.
//! Does not own: what disabling does (the workspace), where the choice or the values are stored,
//! the settings host and its navigation, or the plugins' own pages.
//! Ports used: `PluginToggles` (implemented by the application over `[plugins]`) and
//! `PluginSettingsDirectory` (the source of each plugin's values), and with `user` also
//! `UserPlugins`, `UserPluginsSwitch` (the stored "allow user plugins" choice, which applies
//! at the next start) and `PluginsFolder` (the folder plugin files are put in). Provided: a
//! lazy `SettingsPageSpec` with id `plugins`, built by [`settings_page`].
//! Settings section: none. Globals: none.
//! Invariants: the list shows plugins, never core contributions; a plugin that is required has
//! a switch that cannot be turned off; the form shows only what its source holds, and text typed
//! into it is sent when the selection changes or the page is hidden or closed.

mod page;
mod toggles;
mod user;
mod view;

use std::{rc::Rc, sync::Arc};

use gpui_kit::AppContext as _;
use nocterm_plugin_model::PluginCatalog;
use nocterm_ui::{IconName, PluginSettingsDirectory, SettingsPageSpec};

pub use page::PluginsPage;
pub use toggles::{EnablementError, PluginToggles};
pub use user::{
    AuditLine, Candidate, CapabilityLine, FieldLine, FolderError, HeldBy, InstallRequest,
    MAX_LISTED, PluginsFolder, Replaced, RowState, UserPart, UserPartView, UserPluginError,
    UserPluginRow, UserPlugins, UserPluginsSection, UserPluginsSwitch, UserSelected, UserSession,
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
pub fn settings_page_with_user_plugins(
    catalog: Arc<PluginCatalog>,
    toggles: Rc<dyn PluginToggles>,
    directory: PluginSettingsDirectory,
    session: UserSession,
    switch: Rc<dyn UserPluginsSwitch>,
    folder: Arc<dyn PluginsFolder>,
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
            .with_user_plugins(session.clone(), switch.clone(), folder.clone(), cx)
        })
    })
    .with_icon(IconName::Plug)
}

#[cfg(test)]
mod tests;
