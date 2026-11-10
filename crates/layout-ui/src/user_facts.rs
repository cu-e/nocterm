//! What the host knows about an installed user plugin, which the tooltip of its items shows:
//! the facts are the host's own, never taken from the plugin's text.

use std::rc::Rc;

use gpui_kit::App;
use nocterm_plugin_model::PluginId;

/// The facts of one user plugin, worded by the host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserFacts {
    /// Who published it, as the package says. Unverified.
    pub publisher: String,
    /// The short form of the package hash the user compared at install.
    pub fingerprint: String,
    /// What the user allowed the plugin to do, in the host's words.
    pub capabilities: Vec<String>,
}

/// Where the editor reads the facts of user plugins from.
pub trait UserPluginFacts: 'static {
    /// The facts of `plugin`, if it is installed.
    fn facts(&self, plugin: &PluginId, cx: &App) -> Option<UserFacts>;
}

pub(crate) type SharedFacts = Rc<dyn UserPluginFacts>;
