//! What the views of this crate need from the application, named for the consumer.

use gpui_kit::{App, Window};
use nocterm_core::error::SharedCause;
use nocterm_hubs_model::{Environment, HubError, HubId};

/// Where an environment is opened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    /// Instead of what the window shows now.
    ThisWindow,
    /// In a window of its own; the window asking is left alone.
    NewWindow,
}

/// Why an environment could not be opened.
#[derive(Debug, thiserror::Error)]
pub enum EnvironmentOpenError {
    #[error("no such server: {0}")]
    UnknownHub(HubId),
    #[error("the account of the server cannot be opened")]
    Account(#[source] HubError),
    #[error("the window could not be opened")]
    Window(#[source] SharedCause),
}

/// Opens an environment in a window. The application implements it: only it can compose a
/// window.
pub trait EnvironmentOpener {
    /// Opens `environment` in `placement`. [`Placement::ThisWindow`] replaces the window `window`
    /// shows; the replacement may happen after this call returns.
    fn open(
        &self,
        environment: Environment,
        placement: Placement,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<(), EnvironmentOpenError>;
}
