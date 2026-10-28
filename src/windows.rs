//! Windows of the application: each shows one environment, chosen when it is composed.
//!
//! A window is this computer or one hub account. Composing a window gives its workspace the
//! features of the environment's sources and puts the focus in it, so that actions of the
//! window (such as unlocking its vault) reach it.

use gpui_kit::{App, AppContext as _, Entity, Focusable as _, Window};
use nocterm_workspace::Workspace;

use crate::bootstrap::{Booted, build_workspace};
use crate::window_sources::WindowSources;

/// Makes the workspace of a window from `sources` and focuses it.
///
/// A hub window whose account's vault is locked asks for it at once: nothing of the account is
/// readable before.
pub(crate) fn compose(
    booted: &Booted,
    sources: &WindowSources,
    window: &mut Window,
    cx: &mut App,
) -> Entity<Workspace> {
    let workspace = cx.new(|cx| {
        let mut workspace = Workspace::new(booted.workspace_deps(), window, cx);
        build_workspace(&mut workspace, booted, sources, window, cx);
        workspace
    });
    window.focus(&workspace.focus_handle(cx), cx);
    if !sources.environment().is_local()
        && sources
            .vault()
            .is_some_and(|vault| vault.service().status() == nocterm_vault::VaultStatus::Locked)
    {
        window.defer(cx, |window, cx| {
            window.dispatch_action(Box::new(nocterm_workspace::UnlockVault), cx);
        });
    }
    workspace
}
