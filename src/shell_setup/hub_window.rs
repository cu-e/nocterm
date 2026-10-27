//! The workspace of a hub window: the terminal sessions of one server and the features that do
//! not depend on the environment.
//!
//! Connections, files, containers, workspaces, the monitor, AI agents and user plugins are not
//! registered, and neither is the session factory: a hub window opens no local or SSH tab. Tabs
//! come from the sessions panel and the "+" menu, through [`AppHubTerminals`].

use std::{rc::Rc, sync::Arc};

use gpui_kit::{App, Context, SharedString, Window, WindowId};
use nocterm_hubs_model::{HubId, HubWindowMemory};
use nocterm_hubs_ui::{HubMemoryKeeper, HubTabError, HubTerminalOpener, HubWindowPorts};
use nocterm_session::{Target, Transport};
use nocterm_workspace::Workspace;

use crate::{
    bootstrap::{Booted, Windowing},
    windows::AppEnvironmentOpener,
};

/// Plugs the features of a hub window into `workspace`. `restored` is what the window kept of
/// the hub before a restart.
pub(crate) fn build_hub_workspace(
    workspace: &mut Workspace,
    booted: &Booted,
    hub: &HubId,
    restored: &HubWindowMemory,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    // The sessions go first: of the panels in the sidebar the first registered is shown.
    if let Some(hubs) = booted.hubs() {
        let ports = HubWindowPorts {
            environments: AppEnvironmentOpener::new(booted.clone()),
            terminals: Rc::new(AppHubTerminals {
                terminal: booted.terminal().clone(),
            }),
            memory: Rc::new(AppHubMemory {
                windowing: booted.windowing().clone(),
                window: window.window_handle().window_id(),
            }),
        };
        if let Err(error) = nocterm_hubs_ui::register_hub_window(
            hubs,
            hub.clone(),
            ports,
            restored.clone(),
            workspace,
            window,
            cx,
        ) {
            tracing::error!(%error, "the hub window could not be composed");
        }
    }
    booted.register_settings(workspace);
    booted.register_snippets(workspace, window, cx);
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

/// Keeps what a hub window holds of its server in the list of open windows.
struct AppHubMemory {
    windowing: Windowing,
    window: WindowId,
}

impl HubMemoryKeeper for AppHubMemory {
    fn keep(&self, hub: &HubId, memory: HubWindowMemory, cx: &mut App) {
        self.windowing.remembered(self.window, hub, memory, cx);
    }
}
