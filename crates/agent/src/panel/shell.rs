use super::menu_view::MenuView;
use super::widgets::HEADER_HEIGHT;
use super::{AgentPanel, MenuKind, widgets};
use gpui_kit::{
    Anchor, App, Context, FocusHandle, SharedString, TestSupportExt as _,
    component::{
        ActiveTheme as _, Selectable as _, Sizable as _,
        button::{Button, ButtonVariants as _},
        floating::FloatingCards,
        h_flex, v_flex,
    },
    div,
    prelude::*,
    px,
};
use nocterm_ui::IconName;
use nocterm_workspace::RightPanelEvent;

pub(super) struct ShellRenderer;
impl ShellRenderer {
    pub(super) fn body(
        focus: &FocusHandle,
        cx: &mut Context<AgentPanel>,
    ) -> gpui_kit::base::ObservedElement<gpui_kit::Stateful<gpui_kit::Div>> {
        let body = v_flex()
            .id("agent-panel")
            .test_support()
            .key_context("AgentPanel")
            .track_focus(focus)
            .size_full()
            .min_w_0()
            // A floating card brings the panel's fill.
            .when(FloatingCards::get(cx).is_none(), |body| {
                body.bg(cx.theme().background)
            })
            .capture_action(cx.listener(
                |this, _: &gpui_kit::component::input::MoveUp, window, cx| {
                    this.command_action("up", window, cx);
                },
            ))
            .capture_action(cx.listener(
                |this, _: &gpui_kit::component::input::MoveDown, window, cx| {
                    this.command_action("down", window, cx);
                },
            ))
            .capture_action(cx.listener(
                |this, _: &gpui_kit::component::input::IndentInline, window, cx| {
                    this.command_action("tab", window, cx);
                },
            ))
            .capture_action(cx.listener(
                |this, _: &gpui_kit::component::input::OutdentInline, window, cx| {
                    this.command_action("shift-tab", window, cx);
                },
            ))
            .capture_action(cx.listener(
                |this, _: &gpui_kit::component::input::Escape, window, cx| {
                    if this.dismiss_usage(cx) {
                        cx.stop_propagation();
                    } else {
                        this.command_action("escape", window, cx);
                    }
                },
            ))
            .capture_action(cx.listener(
                |this, action: &gpui_kit::component::input::Enter, window, cx| {
                    if !action.shift && !action.secondary {
                        this.command_action("enter", window, cx);
                    }
                },
            ));
        Self::chat_actions(body, cx)
    }
    fn chat_actions(
        body: gpui_kit::base::ObservedElement<gpui_kit::Stateful<gpui_kit::Div>>,
        cx: &mut Context<AgentPanel>,
    ) -> gpui_kit::base::ObservedElement<gpui_kit::Stateful<gpui_kit::Div>> {
        body.on_action(
            cx.listener(|this, _: &crate::NewThread, _, cx| this.toggle_menu(MenuKind::Agents, cx)),
        )
        .on_action(
            cx.listener(|this, _: &crate::NewThreadWithLastAgent, window, cx| {
                this.new_thread_with_last(window, cx);
            }),
        )
        .on_action(cx.listener(|this, _: &crate::ShowHistory, _, cx| this.toggle_history(cx)))
        .on_action(cx.listener(|this, _: &crate::StopGeneration, _, cx| {
            this.stop_generation(cx);
        }))
        .on_action(cx.listener(|this, _: &crate::ToggleModelPicker, _, cx| {
            this.toggle_model_picker(cx);
        }))
        .on_action(cx.listener(|this, _: &crate::AttachImage, _, cx| this.pick_images(cx)))
    }

    pub(super) fn header(
        agent: Option<gpui_kit::component::Icon>,
        title: SharedString,
        history: bool,
        maximized: bool,
        menus: &MenuView,
        cx: &mut Context<AgentPanel>,
    ) -> gpui_kit::AnyElement {
        h_flex()
            .id("agent-header")
            .test_support()
            .flex_shrink_0()
            // As tall as the workspace's tab strip, so the two line up.
            .h(px(HEADER_HEIGHT))
            .gap_1()
            .pl_3()
            .pr_1()
            .border_b_1()
            .border_color(cx.theme().border)
            .text_sm()
            .child({
                match agent {
                    Some(icon) => icon.small(),
                    None => gpui_kit::component::Icon::new(IconName::Bot)
                        .small()
                        .text_color(cx.theme().muted_foreground),
                }
            })
            .child(Self::title(title, cx))
            .child(
                Button::new("agent-history")
                    .ghost()
                    .small()
                    .icon(IconName::Clock)
                    .tooltip("Chat history")
                    .selected(history)
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_history(cx))),
            )
            .child(super::menu::ConfigMenuRenderer::popover(
                menus,
                "agent-new-popover",
                MenuKind::Agents,
                Button::new("agent-new-thread")
                    .ghost()
                    .small()
                    .icon(IconName::Plus)
                    .tooltip("New chat"),
                Anchor::TopRight,
                cx,
            ))
            .child(
                Button::new("agent-maximize")
                    .ghost()
                    .small()
                    .icon(if maximized {
                        IconName::Minimize2
                    } else {
                        IconName::Maximize2
                    })
                    .tooltip(if maximized {
                        "Restore panel"
                    } else {
                        "Expand chat"
                    })
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(RightPanelEvent::ToggleMaximized))),
            )
            .into_any_element()
    }
    fn title(
        title: SharedString,
        _cx: &mut Context<AgentPanel>,
    ) -> gpui_kit::base::ObservedElement<gpui_kit::Stateful<gpui_kit::Div>> {
        div()
            .id("agent-chat-title")
            .test_support()
            .flex_1()
            .min_w_0()
            .truncate()
            .tooltip({
                let title = title.clone();
                move |_, cx| {
                    cx.new(|_| gpui_kit::component::tooltip::Tooltip::new(title.clone()))
                        .into()
                }
            })
            .child(widgets::single_line_label(&title.clone()))
    }
    pub(super) fn error(error: impl Into<SharedString>, cx: &App) -> gpui_kit::Div {
        div()
            .p_2()
            .text_sm()
            .text_color(cx.theme().danger)
            .child(error.into())
    }
}
