//! Adapts feature Items to the toolkit's single authoritative pane tree.
use std::rc::Rc;

use crate::{ItemHandle, TabState, Workspace};
use gpui_kit::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, Subscription, WeakEntity, Window,
    base::TestSupportExt as _,
    component::{
        ActiveTheme as _, Icon, Sizable as _,
        button::{Button, ButtonVariants as _},
        dock::{BasePanel, Panel, PanelEvent},
        h_flex,
        input::{Input, InputEvent, InputState},
        menu::{ContextMenuExt as _, PopupMenuItem},
    },
    div,
    prelude::*,
    rems,
};
use nocterm_ui::ActiveDesign as _;

pub(crate) struct DockItem {
    pub(crate) item: Rc<dyn ItemHandle>,
    workspace: WeakEntity<Workspace>,
    pub(crate) alias: Option<gpui_kit::SharedString>,
    editing: Option<Entity<InputState>>,
    editing_subscription: Option<Subscription>,
    editing_blur: Option<Subscription>,
    focus: FocusHandle,
    bottom: bool,
    _subscriptions: Vec<Subscription>,
}
impl EventEmitter<PanelEvent> for DockItem {}
impl DockItem {
    /// Focus membership belongs to the whole view, while `Focusable` chooses
    /// the feature's preferred control when activating it.
    pub(crate) fn container_focus_handle(&self) -> FocusHandle {
        self.focus.clone()
    }
    pub(crate) fn contains_focus(&self, window: &Window, cx: &App) -> bool {
        self.focus.contains_focused(window, cx)
    }
    pub(crate) fn new(
        item: Rc<dyn ItemHandle>,
        workspace: WeakEntity<Workspace>,
        bottom: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        let subscriptions = vec![cx.on_focus_in(&focus, window, |this, window, cx| {
            if !this.bottom {
                this.announce(window, cx);
            }
        })];
        Self {
            item,
            workspace,
            alias: None,
            editing: None,
            editing_subscription: None,
            editing_blur: None,
            focus,
            bottom,
            _subscriptions: subscriptions,
        }
    }
    fn announce(&self, window: &Window, cx: &mut Context<Self>) {
        let workspace = self.workspace.clone();
        let id = self.item.item_id();
        cx.defer_in(window, move |_, _, cx| {
            let _ = workspace.update(cx, |workspace, cx| workspace.mark_active(id, cx));
        });
    }
    pub(crate) fn start_alias(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.editing.is_some() {
            return;
        }
        let title = self
            .alias
            .clone()
            .unwrap_or_else(|| self.item.tab_title(cx));
        let input = cx.new(|cx| InputState::new(window, cx).default_value(title));
        self.editing_subscription =
            Some(
                cx.subscribe_in(&input, window, |this, _, event, window, cx| {
                    if matches!(event, InputEvent::PressEnter { .. }) {
                        this.finish_alias(true, true, window, cx);
                    }
                }),
            );
        let focus = input.read(cx).focus_handle(cx);
        self.editing_blur = Some(cx.on_blur(&focus, window, |this, window, cx| {
            this.finish_alias(true, false, window, cx)
        }));
        self.editing = Some(input);
        window.focus(&focus, cx);
        cx.notify();
    }
    fn finish_alias(
        &mut self,
        accept: bool,
        restore_focus: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(input) = self.editing.take() {
            self.editing_subscription.take();
            self.editing_blur.take();
            if accept {
                let value = input.read(cx).value().trim().to_string();
                self.alias = (!value.is_empty()).then(|| value.into());
            }
            if restore_focus {
                window.focus(&self.item.focus_handle(cx), cx);
            }
            cx.emit(PanelEvent::LayoutChanged);
            cx.notify();
        }
    }
}
impl Focusable for DockItem {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.item.focus_handle(cx)
    }
}
impl BasePanel for DockItem {
    fn panel_name(&self) -> &'static str {
        "nocterm.item"
    }
    fn on_removed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let workspace = self.workspace.clone();
        let id = self.item.item_id();
        let bottom = self.bottom;
        cx.defer_in(window, move |_, window, cx| {
            let _ = workspace.update(cx, |workspace, cx| {
                workspace.item_removed(id, bottom, window, cx)
            });
        });
    }
    fn set_active(&mut self, active: bool, window: &mut Window, cx: &mut Context<Self>) {
        if active && !self.bottom && self.contains_focus(window, cx) {
            self.announce(window, cx);
        }
    }
}
impl Panel for DockItem {
    fn inner_padding(&self, _: &App) -> bool {
        false
    }
    fn title(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let icon_color = match self.item.tab_state(cx) {
            TabState::Idle => cx.theme().muted_foreground,
            TabState::Busy => cx.theme().info,
            TabState::Attention => cx.theme().warning,
            TabState::Ended => cx.theme().danger,
        };
        let workspace = self.workspace.clone();
        let menu_workspace = workspace.clone();
        let id = self.item.item_id();
        let bottom = self.bottom;
        h_flex()
            .id(("tab-title", id.as_u64()))
            .test_support()
            .pl_2()
            .pr_1()
            .gap_2()
            .min_w_0()
            .max_w(rems(cx.design().layout.tab_max_width))
            .child(
                Icon::new(self.item.tab_icon(cx))
                    .small()
                    .text_color(icon_color),
            )
            .child(match &self.editing {
                Some(input) => div()
                    .key_context("TabAlias")
                    .w(rems(cx.design().layout.tab_max_width - 3.0))
                    .child(Input::new(input).small())
                    .on_key_down(
                        cx.listener(|this, event: &gpui_kit::KeyDownEvent, window, cx| {
                            if event.keystroke.key == "escape" {
                                cx.stop_propagation();
                                this.finish_alias(false, true, window, cx);
                            }
                        }),
                    )
                    .into_any_element(),
                None => div()
                    .id("tab-alias")
                    .min_w_0()
                    .text_ellipsis()
                    .child(
                        self.alias
                            .clone()
                            .unwrap_or_else(|| self.item.tab_title(cx)),
                    )
                    .on_click(
                        cx.listener(|this, event: &gpui_kit::ClickEvent, window, cx| {
                            if event.click_count() == 2 {
                                cx.stop_propagation();
                                this.start_alias(window, cx);
                            }
                        }),
                    )
                    .into_any_element(),
            })
            .child(
                Button::new(("close-tab", id.as_u64()))
                    .ghost()
                    .xsmall()
                    .icon(nocterm_ui::IconName::X)
                    .tooltip("Close Tab")
                    .on_click(move |_, window, cx| {
                        cx.stop_propagation();
                        let _ = workspace.update(cx, |this, cx| {
                            if bottom {
                                this.close_local_terminal(window, cx);
                            } else {
                                let ix = this.items().position(|item| item.item_id() == id);
                                if let Some(ix) = ix {
                                    this.close_item(ix, window, cx);
                                }
                            }
                        });
                    }),
            )
            .context_menu(move |mut menu, _, cx| {
                use crate::workspace::TabCloseScope;
                if bottom {
                    let workspace = menu_workspace.clone();
                    return menu
                        .item(PopupMenuItem::new("Close Terminal").on_click(
                            move |_, window, cx| {
                                let _ = workspace.update(cx, |workspace, cx| {
                                    workspace.close_local_terminal(window, cx)
                                });
                            },
                        ))
                        .separator()
                        .menu("Settings", Box::new(crate::OpenSettings));
                }
                for (label, scope) in [
                    ("Close Tab", TabCloseScope::Current),
                    ("Close Other Tabs in Pane", TabCloseScope::Others),
                    ("Close Tabs to the Left", TabCloseScope::Left),
                    ("Close Tabs to the Right", TabCloseScope::Right),
                    ("Close All Tabs", TabCloseScope::All),
                ] {
                    let disabled = menu_workspace.upgrade().is_none_or(|workspace| {
                        workspace.read(cx).tabs_to_close(id, scope, cx).is_empty()
                    });
                    let workspace = menu_workspace.clone();
                    menu = menu.item(PopupMenuItem::new(label).disabled(disabled).on_click(
                        move |_, window, cx| {
                            let _ = workspace.update(cx, |workspace, cx| {
                                workspace.close_tabs(id, scope, window, cx)
                            });
                        },
                    ));
                }
                menu = menu.separator();
                for (label, placement) in [
                    (
                        "Split Vertically — Right",
                        gpui_kit::component::Placement::Right,
                    ),
                    (
                        "Split Horizontally — Below",
                        gpui_kit::component::Placement::Bottom,
                    ),
                ] {
                    let workspace = menu_workspace.clone();
                    menu = menu.item(PopupMenuItem::new(label).on_click(move |_, window, cx| {
                        let _ = workspace.update(cx, |workspace, cx| {
                            workspace.split_item(id, placement, window, cx)
                        });
                    }));
                }
                menu.separator()
                    .menu("Settings", Box::new(crate::OpenSettings))
            })
    }
}
impl Render for DockItem {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .track_focus(&self.focus)
            .child(self.item.view())
    }
}
