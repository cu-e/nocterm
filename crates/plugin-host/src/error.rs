//! Typed errors of the host and of its two storage ports.

use nocterm_plugin_api::{
    Cause, EffectsError, GrantError, HashError, HostError, LimitError, PackageError, RuntimeError,
    ViewError,
};
use nocterm_plugin_model::PluginId;
use thiserror::Error;

/// A document that is not valid JSON of this host, or is outside its bounds.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum DocumentError {
    #[error("not a valid plugin document")]
    Syntax(#[source] Cause),
    #[error("a plugin document is outside its bounds")]
    Bounds(#[from] DataBoundsError),
}

impl DocumentError {
    pub(crate) fn syntax(error: serde_json::Error) -> Self {
        Self::Syntax(Cause::new(error))
    }
}

/// Why the records could not be saved.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum RecordsError {
    /// The application is shutting down, so nothing more is written. Not a loss of data the
    /// user could still act on.
    #[error("the plugin records were not saved because persistence is closing")]
    Closing,
    #[error("the plugin records could not be saved")]
    Write(#[source] Cause),
}

/// Why a plugin's data document could not be read.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum PluginDataError {
    /// The file exists but is not a document of this host. It stays as it is until the user
    /// changes something, which replaces it.
    #[error("the plugin data could not be read")]
    Broken(#[source] Cause),
    /// The file is a document of this host but outside its limits. Treated like a broken file.
    #[error("the plugin data is outside its limits")]
    Bounds(#[from] DataBoundsError),
    #[error("the plugin data could not be accessed")]
    Unreadable(#[source] Cause),
    #[error("`{0}` is not a plugin id")]
    NotAPluginId(PluginId),
    /// The change was not queued, for example because persistence is closing.
    #[error("the plugin data change was not accepted for saving")]
    NotSaved(#[source] Cause),
}

/// A plugin's data outside its limits.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum DataBoundsError {
    #[error("the setting values are larger than {max} bytes")]
    ValuesTooLarge { max: usize },
    #[error("the plugin storage is larger than {max} bytes")]
    StorageTooLarge { max: usize },
    #[error("a stored entry is outside its limits")]
    Entry(#[from] LimitError),
}

/// Why a stored plugin is listed but does not run.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum BrokenPackage {
    #[error(transparent)]
    Hash(#[from] HashError),
    #[error(transparent)]
    Package(#[from] PackageError),
    #[error("the stored package belongs to another plugin")]
    OtherPlugin { found: PluginId },
    #[error(transparent)]
    Grant(#[from] GrantError),
}

/// Why a change to the installed plugins was refused.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum ManageError {
    #[error("`{0}` is not installed")]
    Unknown(PluginId),
    #[error("`{0}` is already installed")]
    Installed(PluginId),
    #[error("a replacement must keep the plugin id `{0}`")]
    OtherPlugin(PluginId),
    #[error("at most {max} user plugins can be enabled")]
    TooManyEnabled { max: usize },
    #[error("the package could not be stored")]
    Package(#[from] PackageError),
    #[error("the consent does not fit the package")]
    Grant(#[from] GrantError),
    #[error("the plugin cannot run")]
    Runtime(#[from] RuntimeError),
    #[error("the plugin data is outside its limits")]
    Data(#[from] DataBoundsError),
    #[error("the plugin data could not be saved")]
    Unsaved(#[from] PluginDataError),
}

/// Why a render or an event did not run.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum CallError {
    #[error("`{0}` is not installed")]
    Unknown(PluginId),
    /// The plugin is disabled, faulted, or its package no longer matches what the user accepted.
    #[error("`{0}` is not running")]
    NotRunning(PluginId),
    #[error("the package has no contribution `{0}`")]
    NoContribution(String),
    /// Too many renders this second across user plugins; the caller keeps its last view.
    #[error("too many renders, retry after {retry_after_ms} ms")]
    Throttled { retry_after_ms: u32 },
    #[error("the plugin failed")]
    Runtime(#[from] RuntimeError),
    #[error("the plugin drew an invalid view")]
    View(#[from] ViewError),
    #[error("the plugin asked for effects the host refuses")]
    Effects(#[from] EffectsError),
}

impl From<CallError> for HostError {
    fn from(error: CallError) -> Self {
        match error {
            CallError::Throttled { retry_after_ms } => Self::RateLimited { retry_after_ms },
            _ => Self::Unavailable,
        }
    }
}
