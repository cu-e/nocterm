//! Composing the hubs of the window: the commands that sign in to and out of a server.

use gpui_kit::AppContext as _;
use nocterm_workspace::Workspace;

use crate::installation::Installed;
use crate::{SignInToServer, SignOutOfServer, commands, sign_in};

/// Registers the commands [`SignInToServer`] and [`SignOutOfServer`] of `installed` in
/// `workspace`.
pub fn register(installed: &Installed, workspace: &mut Workspace) {
    let hubs = installed.hubs().clone();
    let sign_in_hubs = hubs.clone();
    workspace.register_action(move |_, _: &SignInToServer, window, cx| {
        sign_in::start(&sign_in_hubs, None, window, cx);
    });
    workspace.register_action(move |_, _: &SignOutOfServer, window, cx| {
        let hubs = hubs.clone();
        window.defer(cx, move |window, cx| {
            commands::pick_server_to_sign_out(&hubs, window, cx);
        });
    });
}
