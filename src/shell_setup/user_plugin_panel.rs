//! How a window draws user plugins: a panel or a status bar widget per contribution, the
//! provider that lets a plugin badge its own panel, and the registrations that follow
//! the installed plugin set.
//!
//! A contribution is a view the host renders on demand ([`content`]). The window never runs
//! plugin code on its own thread: it asks the host, which validates what comes back, and draws
//! it inside the plugin frame ([`nocterm_plugin_view`]).
use std::{cell::RefCell, rc::Rc};

use gpui_kit::{Context, Entity, Window};
use nocterm_plugin_host::PluginHost;
use nocterm_plugin_ports::PluginProviders;
use nocterm_ui::PluginSetHandle;
use nocterm_workspace::Workspace;

mod content;
mod controls;
mod sync;
mod views;

pub(super) use controls::Badges;
use sync::WindowSync;

/// Registers the user contributions of the plugin set in force and registers again whenever the
/// set changes, for the life of the window.
pub(super) fn attach(
    host: Entity<PluginHost>,
    source: PluginSetHandle,
    providers: PluginProviders,
    badges: Badges,
    workspace: &mut Workspace,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let sync = Rc::new(RefCell::new(WindowSync::new(
        host,
        source.clone(),
        providers,
        badges,
        window.window_handle().window_id(),
    )));
    sync.borrow_mut().run(workspace, window, cx);
    let subscription = sync::follow(sync, &source, cx.weak_entity(), window.window_handle(), cx);
    cx.on_release(move |_, _| drop(subscription)).detach();
}
