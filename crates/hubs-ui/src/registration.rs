//! Composing the hubs of the window: the commands that sign in to and out of a server and the
//! sessions panel.

use std::rc::Rc;

use gpui_kit::{AppContext as _, Context, Window};
use nocterm_workspace::{PanelRegistration, RegistrationError, Workspace};

use crate::installation::Installed;
use crate::ports::HubTerminalOpener;
use crate::sessions::SessionsPanel;
use crate::{OpenHubSessions, SignInToServer, SignOutOfServer, commands, plugin, sign_in};

/// Registers the hubs of `installed` in `workspace`: the commands [`SignInToServer`] and
/// [`SignOutOfServer`], the action [`OpenHubSessions`] and the sessions panel. Tabs of the
/// sessions open through `terminals`.
///
/// The actions are registered even when the workspace refuses the panel.
pub fn register(
    installed: &Installed,
    terminals: Rc<dyn HubTerminalOpener>,
    workspace: &mut Workspace,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> Result<(), RegistrationError> {
    let hubs = installed.hubs().clone();
    let sign_in_hubs = hubs.clone();
    workspace.register_action(move |_, _: &SignInToServer, window, cx| {
        sign_in::start(&sign_in_hubs, None, window, cx);
    });
    let sign_out_hubs = hubs.clone();
    workspace.register_action(move |_, _: &SignOutOfServer, window, cx| {
        let hubs = sign_out_hubs.clone();
        window.defer(cx, move |window, cx| {
            commands::pick_server_to_sign_out(&hubs, window, cx);
        });
    });
    workspace.register_action(|workspace, _: &OpenHubSessions, window, cx| {
        workspace.toggle_panel(&plugin::SESSIONS.id, window, cx);
    });
    workspace.register_panel(
        PanelRegistration::new(&plugin::SESSIONS, move |_, _, cx| {
            let workspace = cx.entity();
            let (hubs, terminals) = (hubs.clone(), Rc::clone(&terminals));
            cx.new(|cx| SessionsPanel::new(hubs, terminals, &workspace, cx))
        }),
        window,
        cx,
    )
}
