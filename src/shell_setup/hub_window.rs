//! The workspace of a hub window: the terminal sessions of one server and the features that do
//! not depend on the environment.
//!
//! Connections, files, containers, workspaces, the monitor, AI agents and user plugins are not
//! registered, and neither is the session factory: a hub window opens no local or SSH tab. Tabs
//! come from the sessions panel and the "+" menu, through [`AppHubTerminals`].

use std::{rc::Rc, sync::Arc};

use gpui_kit::{Context, SharedString, Window};
use nocterm_hubs_model::HubId;
use nocterm_hubs_ui::{HubTabError, HubTerminalOpener};
use nocterm_session::{Target, Transport};
use nocterm_workspace::Workspace;

use crate::{bootstrap::Booted, windows::AppEnvironmentOpener};

/// Plugs the features of a hub window into `workspace`.
pub(crate) fn build_hub_workspace(
    workspace: &mut Workspace,
    booted: &Booted,
    hub: &HubId,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    booted.register_settings(workspace);
    booted.register_snippets(workspace, window, cx);
    if let Some(hubs) = booted.hubs() {
        let terminals = Rc::new(AppHubTerminals {
            terminal: booted.terminal().clone(),
        });
        if let Err(error) = nocterm_hubs_ui::register_hub_window(
            hubs,
            hub.clone(),
            AppEnvironmentOpener::new(booted.clone()),
            terminals,
            workspace,
            window,
            cx,
        ) {
            tracing::error!(%error, "the hub window could not be composed");
        }
    }
    workspace.set_menu_builder(crate::app_menus::build_hub, window, cx);
}

/// Opens hub sessions as terminal tabs.
struct AppHubTerminals {
    terminal: nocterm_terminal::Installed,
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
