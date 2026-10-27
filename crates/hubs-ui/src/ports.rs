//! What the views of this crate need from the application, named for the consumer.

use std::sync::Arc;

use gpui_kit::{App, Context, SharedString, Window};
use nocterm_core::error::SharedCause;
use nocterm_hubs_model::{Environment, HubId};
use nocterm_session::{Target, Transport};
use nocterm_workspace::Workspace;
use nocterm_workspace_ports::ContextSelectionError;

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

/// A hub tab could not be opened.
#[derive(Debug, thiserror::Error)]
#[error("the tab could not be opened")]
pub struct HubTabError(#[from] pub ContextSelectionError);

/// Opens a terminal tab attached to a hub session. The application implements it with the
/// terminal feature; features do not depend on each other.
pub trait HubTerminalOpener {
    /// Adds a tab titled `title` on `transport` to `workspace`. `target` names the hub the tab
    /// belongs to.
    fn open(
        &self,
        workspace: &mut Workspace,
        title: SharedString,
        target: Target,
        transport: Arc<dyn Transport>,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) -> Result<(), HubTabError>;
}
