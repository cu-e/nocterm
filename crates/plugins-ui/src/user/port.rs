//! The port the User section reads and acts through, named for the section's need.

use std::error::Error;

use gpui_kit::{App, Subscription};
use nocterm_core::error::SharedCause;
use nocterm_plugin_model::PluginId;

/// A package the user picked, read and checked but not stored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    /// Where it was read, to read it again at install.
    pub location: String,
    pub id: PluginId,
    pub name: String,
    /// Who published it, as the package says. Unverified.
    pub publisher: String,
    pub version: String,
    /// The sha256 of the bytes that were checked, in hexadecimal.
    pub hash: String,
    /// The short form of the hash the user compares.
    pub fingerprint: String,
    pub capabilities: Vec<CapabilityLine>,
    /// The fields the package lets the user set.
    pub fields: Vec<FieldLine>,
    /// What an installed plugin of the same id is, when this replaces it.
    pub replaces: Option<Replaced>,
    /// The package is a code module the plugin runs in a sandbox, not a declarative template.
    pub runs_code: bool,
    /// The package comes from a developer session: the plugin, its data and settings, and the
    /// application's settings are not saved.
    pub dev: bool,
}

/// One capability the package asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapabilityLine {
    pub id: String,
    /// What it lets the plugin do, written by the host.
    pub description: String,
    /// Why the author says it needs it. Unverified.
    pub reason: Option<String>,
    pub required: bool,
    /// New since the installed version.
    pub added: bool,
}

/// One field of the package's settings form.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldLine {
    pub label: String,
    /// New since the installed version.
    pub added: bool,
}

/// The installed plugin a candidate would replace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Replaced {
    pub version: String,
    /// The sha256 of the installed package, in hexadecimal.
    pub hash: String,
    pub fingerprint: String,
    /// The candidate is an older version.
    pub downgrade: bool,
    /// The installed package is a code module.
    pub runs_code: bool,
}

/// What the user decided at the consent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstallRequest {
    pub location: String,
    /// The hash the user saw. A file that changed since is refused.
    pub hash: String,
    /// The hash of the installed package the user agreed to replace, or `None` for a fresh
    /// install. Refused when the installed package is no longer that one, for example because
    /// another window replaced or removed it meanwhile.
    pub replaces: Option<String>,
    /// The optional capabilities the user allowed.
    pub accepted: Vec<String>,
    /// Keep the data of the plugin being replaced.
    pub keep_data: bool,
}

/// Whether an installed plugin can run.
#[derive(Clone, Debug)]
pub enum RowState {
    Ready,
    /// Three traps or missed deadlines this session.
    Faulted,
    /// The stored package is not what the user accepted, or cannot run. The cause keeps its
    /// own sources for [`describe`].
    Broken(SharedCause),
}

/// One installed user plugin.
#[derive(Clone, Debug)]
pub struct UserPluginRow {
    pub id: PluginId,
    pub name: String,
    pub publisher: String,
    pub version: String,
    pub fingerprint: String,
    pub state: RowState,
    pub enabled: bool,
    pub placeable: bool,
    /// What the user allowed it to do, as the host words it.
    pub granted: Vec<String>,
    /// The titles of its panels and widgets.
    pub contributions: Vec<String>,
}

/// One line of the audit log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuditLine {
    pub plugin: PluginId,
    pub action: String,
}

/// An action the application refused. [`describe`] words it, with its causes, for the user.
#[derive(Clone, Debug, thiserror::Error)]
pub enum UserPluginError {
    /// The file is not the one the user looked at.
    #[error("The file changed after you looked at it. Open it again.")]
    Changed,
    /// The installed plugin is not the one the user agreed to replace.
    #[error("The installed plugin changed after you looked at it. Open the file again.")]
    InstalledChanged,
    /// The package could not be read or checked.
    #[error("The package was refused")]
    Package(#[source] SharedCause),
    /// The host refused the change.
    #[error("The change was refused")]
    Manage(#[source] SharedCause),
}

/// The words for an error and its causes, outermost first. A shared cause repeats the text of
/// the error it holds, so a frame equal to the one before it is left out.
pub(super) fn describe(error: &(dyn Error + 'static)) -> String {
    let mut text = error.to_string();
    let mut last = text.clone();
    let mut source = error.source();
    while let Some(cause) = source {
        let words = cause.to_string();
        if words != last {
            text.push_str(": ");
            text.push_str(&words);
            last = words;
        }
        source = cause.source();
    }
    text
}

/// The user plugins of the installation.
pub trait UserPlugins: 'static {
    /// The installed plugins, in a stable order.
    fn rows(&self, cx: &App) -> Vec<UserPluginRow>;

    /// What happened to plugins, oldest first.
    fn audit(&self, cx: &App) -> Vec<AuditLine>;

    /// The file a developer session loads (`nocterm plugin dev <file>`), when this is one.
    fn dev_location(&self, _cx: &App) -> Option<String> {
        None
    }

    /// Reads and checks the package at `location`. Nothing is stored.
    fn inspect(&self, location: &str, cx: &App) -> Result<Candidate, UserPluginError>;

    /// Installs, or replaces an installed plugin of the same id, from what the user accepted.
    fn install(&self, request: InstallRequest, cx: &mut App) -> Result<(), UserPluginError>;

    fn set_enabled(
        &self,
        plugin: &PluginId,
        enabled: bool,
        cx: &mut App,
    ) -> Result<(), UserPluginError>;

    /// Allows or revokes the plugin's panels and widgets in the windows.
    fn set_placeable(
        &self,
        plugin: &PluginId,
        placeable: bool,
        cx: &mut App,
    ) -> Result<(), UserPluginError>;

    /// Removes the plugin; its data goes too when `with_data` is set.
    fn remove(
        &self,
        plugin: &PluginId,
        with_data: bool,
        cx: &mut App,
    ) -> Result<(), UserPluginError>;

    /// Removes the data of the plugin and leaves it installed.
    fn remove_data(&self, plugin: &PluginId, cx: &mut App) -> Result<(), UserPluginError>;

    /// Calls `on_change` after anything the rows or the audit show changed.
    fn observe(&self, cx: &mut App, on_change: Box<dyn FnMut(&mut App)>) -> Subscription;
}
