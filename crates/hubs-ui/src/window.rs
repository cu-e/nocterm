//! Composing the workspace of a hub window.

use std::rc::Rc;

use gpui_kit::{AppContext as _, Context, Window};
use nocterm_hubs_model::{Environment, HubId};
use nocterm_workspace::{PanelRegistration, RegistrationError, Workspace};

use crate::installation::Installed;
use crate::menu::install_menu;
use crate::ports::{EnvironmentOpener, HubTerminalOpener};
use crate::sessions::{NewSessionMenu, SessionsPanel};
use crate::tabs::HubTabs;
use crate::{OpenHubSessions, plugin};

/// Why a workspace could not be made a hub window.
#[derive(Debug, thiserror::Error)]
pub enum HubWindowError {
    #[error("no such server: {0}")]
    UnknownHub(HubId),
    #[error("the sessions panel could not be registered")]
    Panel(#[from] RegistrationError),
}

/// Makes `workspace` the window of `hub`: the sessions panel, the menu behind "+" that starts a
/// session on a machine, and the environment menu. Tabs open through `terminals`.
///
/// The action [`OpenHubSessions`] is registered even when the workspace refuses the panel.
pub fn register_hub_window(
    installed: &Installed,
    hub: HubId,
    opener: Rc<dyn EnvironmentOpener>,
    terminals: Rc<dyn HubTerminalOpener>,
    workspace: &mut Workspace,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> Result<(), HubWindowError> {
    let hubs = installed.hubs().clone();
    let Some(state) = hubs.update(cx, |hubs, cx| hubs.window(&hub, cx)) else {
        return Err(HubWindowError::UnknownHub(hub));
    };
    install_menu(
        installed,
        Environment::Hub(hub.clone()),
        opener,
        workspace,
        window,
        cx,
    );
    let tabs = HubTabs::new(hubs.clone(), hub, cx.weak_entity(), terminals);
    let owner = cx.weak_entity();
    let menu = cx.new(|cx| NewSessionMenu::new(state.clone(), tabs.clone(), owner, cx));
    workspace.set_new_tab_menu(menu, cx);
    workspace.register_action(|workspace, _: &OpenHubSessions, window, cx| {
        workspace.toggle_panel(&plugin::SESSIONS.id, window, cx);
    });
    workspace.register_panel(
        PanelRegistration::new(&plugin::SESSIONS, move |workspace, _, cx| {
            let _ = workspace;
            let handle = cx.entity();
            cx.new(|cx| SessionsPanel::new(hubs.clone(), state.clone(), tabs.clone(), &handle, cx))
        }),
        window,
        cx,
    )?;
    Ok(())
}
