//! Typed action registrations and native command categories for the window shell.
use super::{TabCloseScope, Workspace, panel_groups::GroupKey};
use crate::KEY_CONTEXT;
use gpui_kit::{
    Action, Context, Div, FocusHandle, Window,
    component::{ActiveTheme as _, v_flex},
    prelude::*,
};
use std::rc::Rc;

type Registration = Box<dyn Fn(Div, &mut Context<Workspace>) -> Div>;
#[derive(Default)]
pub(super) struct ActionRegistry {
    registrations: Vec<Registration>,
}
impl ActionRegistry {
    pub(super) fn register<A: Action>(
        &mut self,
        handler: impl Fn(&mut Workspace, &A, &mut Window, &mut Context<Workspace>) + 'static,
    ) {
        let handler = Rc::new(handler);
        self.registrations.push(Box::new(move |element, cx| {
            let handler = handler.clone();
            element.on_action(cx.listener(move |workspace, action: &A, window, cx| {
                handler(workspace, action, window, cx)
            }))
        }));
    }
    pub(super) fn bind(&self, focus: &FocusHandle, cx: &mut Context<Workspace>) -> Div {
        let root = Self::tab_commands(focus, cx);
        let mut root = Self::layout_commands(root, cx);
        for register in &self.registrations {
            root = register(root, cx);
        }
        root
    }
    fn tab_commands(focus: &FocusHandle, cx: &mut Context<Workspace>) -> Div {
        let theme = cx.theme();
        v_flex()
            .key_context(KEY_CONTEXT)
            .track_focus(focus)
            .size_full()
            .bg(super::chrome::Chrome::window_background(cx))
            .text_color(theme.foreground)
            .on_action(cx.listener(Workspace::on_new_tab))
            .on_action(cx.listener(Workspace::on_close_tab))
            .on_action(cx.listener(|this, _: &crate::CloseOtherTabs, window, cx| {
                this.close_active_tabs(TabCloseScope::Others, window, cx)
            }))
            .on_action(cx.listener(|this, _: &crate::CloseTabsLeft, window, cx| {
                this.close_active_tabs(TabCloseScope::Left, window, cx)
            }))
            .on_action(cx.listener(|this, _: &crate::CloseTabsRight, window, cx| {
                this.close_active_tabs(TabCloseScope::Right, window, cx)
            }))
            .on_action(cx.listener(|this, _: &crate::CloseAllTabs, window, cx| {
                this.close_active_tabs(TabCloseScope::All, window, cx)
            }))
            .on_action(cx.listener(Workspace::on_next_tab))
            .on_action(cx.listener(Workspace::on_previous_tab))
            .on_action(cx.listener(Workspace::on_toggle_sidebar))
            .on_action(cx.listener(Workspace::on_next_panel))
            .on_action(cx.listener(|this, _: &crate::SwapSides, _, cx| this.swap_sides(cx)))
            .on_action(
                cx.listener(|this, _: &crate::ToggleRightPanel, window, cx| {
                    this.toggle_assistant_group(window, cx)
                }),
            )
            .on_action(
                cx.listener(|this, _: &crate::ToggleRightPanelMaximized, _, cx| {
                    this.toggle_group_expanded(GroupKey::Assistant, cx)
                }),
            )
    }
    fn layout_commands(root: Div, cx: &mut Context<Workspace>) -> Div {
        root.on_action(cx.listener(|this, _: &crate::MoveTabLeft, window, cx| {
            this.move_active_tab(-1, window, cx)
        }))
        .on_action(cx.listener(|this, _: &crate::MoveTabRight, window, cx| {
            this.move_active_tab(1, window, cx)
        }))
        .on_action(cx.listener(|this, _: &crate::SplitLeft, window, cx| {
            this.split_active(gpui_kit::component::Placement::Left, window, cx)
        }))
        .on_action(cx.listener(|this, _: &crate::SplitRight, window, cx| {
            this.split_active(gpui_kit::component::Placement::Right, window, cx)
        }))
        .on_action(cx.listener(|this, _: &crate::SplitUp, window, cx| {
            this.split_active(gpui_kit::component::Placement::Top, window, cx)
        }))
        .on_action(cx.listener(|this, _: &crate::SplitDown, window, cx| {
            this.split_active(gpui_kit::component::Placement::Bottom, window, cx)
        }))
        .on_action(
            cx.listener(|this, _: &crate::NextPane, window, cx| this.focus_pane(1, window, cx)),
        )
        .on_action(
            cx.listener(|this, _: &crate::PreviousPane, window, cx| {
                this.focus_pane(-1, window, cx)
            }),
        )
        .on_action(cx.listener(|this, _: &crate::RenameTab, window, cx| {
            let selection = this.command_selection(window, cx);
            let workspace = cx.weak_entity();
            window.defer(cx, move |window, cx| {
                let _ = workspace.update(cx, |this, cx| {
                    if this.accepts_context_selection(selection, cx) {
                        this.rename_active_tab(window, cx);
                    }
                });
            });
        }))
        .on_action(
            cx.listener(|this, _: &crate::ToggleLocalTerminal, window, cx| {
                this.toggle_local_terminal(window, cx)
            }),
        )
        .on_action(
            cx.listener(|this, _: &crate::CloseLocalTerminal, window, cx| {
                this.close_local_terminal(window, cx)
            }),
        )
    }
}
