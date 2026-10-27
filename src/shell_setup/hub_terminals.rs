//! Opens the terminal sessions of a hub as tabs of a window.

use std::sync::Arc;

use gpui_kit::{Context, SharedString, Window};
use nocterm_hubs_ui::{HubTabError, HubTerminalOpener};
use nocterm_session::{Target, Transport};
use nocterm_workspace::Workspace;

/// The hubs feature asks for a tab; the terminal feature makes it.
pub(crate) struct AppHubTerminals {
    terminal: nocterm_terminal::Installed,
}

impl AppHubTerminals {
    pub(crate) fn new(terminal: nocterm_terminal::Installed) -> Self {
        Self { terminal }
    }
}

impl HubTerminalOpener for AppHubTerminals {
    fn open(
        &self,
        workspace: &mut Workspace,
        title: SharedString,
        target: Target,
        transport: Arc<dyn Transport>,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) -> Result<(), HubTabError> {
        nocterm_terminal::open_attached(
            &self.terminal,
            workspace,
            nocterm_terminal::AttachedSession::new(title.to_string(), target, transport),
            window,
            cx,
        )?;
        Ok(())
    }
}
