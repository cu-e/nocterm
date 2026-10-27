//! What the views of this crate need from the application, named for the consumer.

use std::sync::Arc;

use gpui_kit::{Context, SharedString, Window};
use nocterm_session::{Target, Transport};
use nocterm_workspace::Workspace;
use nocterm_workspace_ports::ContextSelectionError;

/// A hub tab could not be opened.
#[derive(Debug, thiserror::Error)]
#[error("the tab could not be opened")]
pub struct HubTabError(#[from] ContextSelectionError);

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
