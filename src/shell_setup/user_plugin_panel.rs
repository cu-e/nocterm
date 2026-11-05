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
mod driver;
mod sync;
mod views;

#[cfg(test)]
mod tests;

pub(super) use controls::Badges;
pub(super) use driver::RefreshDriver;
pub(super) use sync::WindowSync;

/// Registers the user contributions of the plugin set in force and registers again whenever the
/// set changes, for the life of the window.
pub(super) fn attach(
    sync: WindowSync,
    driver: gpui_kit::Entity<RefreshDriver>,
    workspace: &mut Workspace,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let source = sync.source().clone();
    let deferred = driver.downgrade();
    let visible = driver.clone();
    let visibility = cx.observe_window_activation(window, move |_, window, cx| {
        visible.update(cx, |driver, cx| driver.visibility(window.is_visible(), cx));
    });
    let panels = cx.subscribe(&cx.entity(), move |_, _, event, cx| {
        if matches!(
            event,
            nocterm_workspace::WorkspaceEvent::PanelVisibilityChanged
                | nocterm_workspace::WorkspaceEvent::ContextChanged
        ) {
            let driver = deferred.clone();
            cx.defer(move |cx| {
                let _ = driver.update(cx, |driver, cx| driver.defer_wake(cx));
            });
        }
    });
    let sync = Rc::new(RefCell::new(sync));
    sync.borrow_mut().run(workspace, window, cx);
    let subscription = sync::follow(sync, &source, cx.weak_entity(), window.window_handle(), cx);
    cx.on_release(move |_, cx| {
        driver.update(cx, |driver, cx| driver.close(cx));
        drop((subscription, visibility, panels));
    })
    .detach();
}
