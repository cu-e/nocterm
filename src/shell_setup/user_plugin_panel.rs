//! How a window draws user plugins: a panel or a status bar widget per contribution, the
//! provider that lets a plugin badge its own panel, and the registrations that follow
//! the installed plugin set.
//!
//! A contribution is a view the host renders on demand ([`content`]). The window never runs
//! plugin code on its own thread: it asks the host, which validates what comes back, and draws
//! it inside the plugin frame ([`nocterm_plugin_view`]).
use std::{cell::RefCell, rc::Rc};

use gpui_kit::{Context, Window};
use nocterm_workspace::Workspace;

mod content;
mod controls;
mod sync;
mod views;

pub(super) use controls::Badges;
pub(super) use sync::WindowSync;

/// Registers the user contributions of the plugin set in force and registers again whenever the
/// set changes, for the life of the window.
pub(super) fn attach(
    sync: WindowSync,
    workspace: &mut Workspace,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let source = sync.source().clone();
    let sync = Rc::new(RefCell::new(sync));
    sync.borrow_mut().run(workspace, window, cx);
    let subscription = sync::follow(sync, &source, cx.weak_entity(), window.window_handle(), cx);
    cx.on_release(move |_, _| drop(subscription)).detach();
}
