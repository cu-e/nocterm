//! The focused inner level consumes its own tab commands before the window shell.
use super::*;
use gpui_kit::{Div, component::Placement};

pub(super) fn bind(root: Div, cx: &mut Context<TabSurface>) -> Div {
    layout(tabs(root, cx), cx)
}
fn tabs(root: Div, cx: &mut Context<TabSurface>) -> Div {
    root.on_action(cx.listener(|this, _: &NewTab, _, cx| {
        cx.stop_propagation();
        if this.terminal_creation_allowed() {
            cx.emit(SurfaceEvent::OpenTerminal(this.selection(), None));
        }
    }))
    .on_action(
        cx.listener(|this, _: &CloseTab, window, cx| {
            close(this, TabCloseScope::Current, window, cx)
        }),
    )
    .on_action(cx.listener(|this, _: &crate::CloseOtherTabs, window, cx| {
        close(this, TabCloseScope::Others, window, cx)
    }))
    .on_action(cx.listener(|this, _: &crate::CloseTabsLeft, window, cx| {
        close(this, TabCloseScope::Left, window, cx)
    }))
    .on_action(cx.listener(|this, _: &crate::CloseTabsRight, window, cx| {
        close(this, TabCloseScope::Right, window, cx)
    }))
    .on_action(cx.listener(|this, _: &crate::CloseAllTabs, window, cx| {
        close(this, TabCloseScope::All, window, cx)
    }))
    .on_action(cx.listener(|this, _: &NextTab, window, cx| next(this, 1, false, window, cx)))
    .on_action(cx.listener(|this, _: &PreviousTab, window, cx| next(this, -1, false, window, cx)))
    .on_action(cx.listener(|this, _: &crate::NextPane, window, cx| next(this, 1, true, window, cx)))
    .on_action(
        cx.listener(|this, _: &crate::PreviousPane, window, cx| next(this, -1, true, window, cx)),
    )
}
fn layout(root: Div, cx: &mut Context<TabSurface>) -> Div {
    root.on_action(cx.listener(|this, _: &crate::SplitLeft, window, cx| {
        split(this, Placement::Left, window, cx)
    }))
    .on_action(cx.listener(|this, _: &crate::SplitRight, window, cx| {
        split(this, Placement::Right, window, cx)
    }))
    .on_action(
        cx.listener(|this, _: &crate::SplitUp, window, cx| split(this, Placement::Top, window, cx)),
    )
    .on_action(cx.listener(|this, _: &crate::SplitDown, window, cx| {
        split(this, Placement::Bottom, window, cx)
    }))
    .on_action(
        cx.listener(|this, _: &crate::MoveTabLeft, window, cx| move_tab(this, -1, window, cx)),
    )
    .on_action(
        cx.listener(|this, _: &crate::MoveTabRight, window, cx| move_tab(this, 1, window, cx)),
    )
    .on_action(
        cx.listener(|this, _: &crate::CloseLocalTerminal, window, cx| {
            cx.stop_propagation();
            this.close_local_terminal(window, cx);
        }),
    )
    .on_action(
        cx.listener(|this, _: &crate::ToggleLocalTerminal, window, cx| {
            cx.stop_propagation();
            let focused = id(this, window, cx);
            if !this.toggle_local(focused, window, cx) && this.terminal_creation_allowed() {
                cx.emit(SurfaceEvent::OpenTerminal(
                    this.selection(),
                    Some(crate::LocalTerminalTarget::Bottom),
                ));
            }
        }),
    )
    .on_action(cx.listener(|this, _: &crate::RenameTab, window, cx| {
        cx.stop_propagation();
        let target = this.selection();
        let id = id(this, window, cx);
        let source = cx.weak_entity();
        window.defer(cx, move |window, cx| {
            let _ = source.update(cx, |this, cx| {
                if this.accepts_context_selection(target, cx)
                    && let Some(id) = id
                {
                    this.rename_item(id, window, cx);
                }
            });
        });
    }))
}
fn id(this: &TabSurface, window: &Window, cx: &App) -> Option<EntityId> {
    this.content()
        .command_item(window, cx)
        .or_else(|| this.tabs().active_handle())
        .map(|item| item.item_id())
}
fn close(
    this: &mut TabSurface,
    scope: TabCloseScope,
    window: &mut Window,
    cx: &mut Context<TabSurface>,
) {
    cx.stop_propagation();
    if let Some(id) = id(this, window, cx) {
        this.close_tabs(id, scope, window, cx);
    }
}
fn split(
    this: &mut TabSurface,
    placement: Placement,
    window: &mut Window,
    cx: &mut Context<TabSurface>,
) {
    cx.stop_propagation();
    if let Some(id) = id(this, window, cx) {
        this.split_item(id, placement, window, cx);
    }
}
fn move_tab(
    this: &mut TabSurface,
    offset: isize,
    window: &mut Window,
    cx: &mut Context<TabSurface>,
) {
    cx.stop_propagation();
    if let Some(id) = id(this, window, cx) {
        this.content_mut().move_item(id, offset, window, cx);
    }
}
fn next(
    this: &mut TabSurface,
    offset: isize,
    pane: bool,
    window: &mut Window,
    cx: &mut Context<TabSurface>,
) {
    cx.stop_propagation();
    if let Some(id) = id(this, window, cx).and_then(|id| {
        if pane {
            this.tabs().next_pane(id, offset, cx)
        } else {
            this.tabs().next_tab(id, offset, cx)
        }
    }) {
        this.activate_item_by_id(id, window, cx);
    }
}
