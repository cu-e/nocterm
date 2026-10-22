//! Adapts feature Items to the toolkit's single authoritative pane tree.
use std::rc::Rc;

use crate::workspace::surface::TabSurface;
use crate::{ContextSelectionReceipt, ItemHandle, ProjectSurfaceReceipt, TabState};
use gpui_kit::{
    App, Context, Entity, EntityId, EventEmitter, FocusHandle, Focusable, Hsla, Subscription,
    WeakEntity, Window,
    base::TestSupportExt as _,
    component::{
        ActiveTheme as _, Disableable as _, Icon, Sizable as _, Theme,
        button::{Button, ButtonVariants as _},
        dock::{BasePanel, Panel, PanelControl, PanelEvent},
        h_flex,
        input::{Input, InputEvent, InputState},
        menu::{ContextMenuExt as _, PopupMenu, PopupMenuItem},
    },
    div,
    prelude::*,
    rems,
};
use nocterm_ui::ActiveDesign as _;

/// The colors groups of tabs are marked with, in the order they are handed out.
const GROUP_COLORS: [fn(&Theme) -> Hsla; 6] = [
    |theme| theme.blue,
    |theme| theme.magenta,
    |theme| theme.cyan,
    |theme| theme.green,
    |theme| theme.yellow,
    |theme| theme.red,
];

/// How many colors groups of tabs are told apart by.
pub(crate) const GROUP_PALETTE: usize = GROUP_COLORS.len();

pub(crate) struct DockItem {
    pub(crate) item: Rc<dyn ItemHandle>,
    workspace: WeakEntity<TabSurface>,
    source: ProjectSurfaceReceipt,
    pub(crate) alias: Option<gpui_kit::SharedString>,
    editing: Option<Entity<InputState>>,
    alias_selection: Option<ContextSelectionReceipt>,
    editing_subscription: Option<Subscription>,
    editing_blur: Option<Subscription>,
    focus: FocusHandle,
    local: bool,
    header_click_origin: Rc<std::cell::Cell<Option<gpui_kit::EntityId>>>,
    /// The color of the group of tabs this one is in, an index into
    /// [`GROUP_COLORS`].
    group_color: Option<usize>,
    project: Option<crate::ProjectSurface>,
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
        workspace: WeakEntity<TabSurface>,
        source: ProjectSurfaceReceipt,
        local: bool,
        header_click_origin: Rc<std::cell::Cell<Option<gpui_kit::EntityId>>>,
        project: Option<crate::ProjectSurface>,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        Self {
            item,
            workspace,
            source,
            alias: None,
            editing: None,
            alias_selection: None,
            editing_subscription: None,
            editing_blur: None,
            focus,
            local,
            header_click_origin,
            group_color: None,
            project,
        }
    }
    pub(crate) fn set_group_color(&mut self, color: Option<usize>, cx: &mut Context<Self>) {
        if self.group_color != color {
            self.group_color = color;
            cx.notify();
        }
    }
    pub(crate) fn group_color(&self) -> Option<usize> {
        self.group_color
    }

    fn selection(&self, cx: &App) -> Option<ContextSelectionReceipt> {
        let workspace = self.workspace.upgrade()?;
        let workspace = workspace.read(cx);
        (workspace.context_source() == self.source).then(|| workspace.context_selection_receipt())
    }
    pub(crate) fn start_alias(
        &mut self,
        selection: ContextSelectionReceipt,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
        self.alias_selection = Some(selection);
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
            if restore_focus && let Some(selection) = self.alias_selection.take() {
                let workspace = self.workspace.clone();
                let focus = self.item.focus_handle(cx);
                window.defer(cx, move |window, cx| {
                    if workspace
                        .read_with(cx, |workspace, cx| {
                            workspace.accepts_context_selection(selection, cx)
                        })
                        .unwrap_or(false)
                    {
                        window.focus(&focus, cx);
                    }
                });
            } else {
                self.alias_selection = None;
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
    fn set_active(&mut self, active: bool, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(project) = &self.project {
            project.set_visible(active, cx);
        }
    }
    fn panel_name(&self) -> &'static str {
        "nocterm.item"
    }
    fn on_removed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let workspace = self.workspace.clone();
        let id = self.item.item_id();
        let source = self.source;
        window.defer(cx, move |window, cx| {
            let _ = workspace.update(cx, |workspace, cx| {
                workspace.item_removed(source, id, window, cx)
            });
        });
    }
}
impl Panel for DockItem {
    fn zoom_control(&self, _: &App) -> Option<PanelControl> {
        Some(if self.local {
            PanelControl::Toolbar
        } else {
            PanelControl::Menu
        })
    }
    fn menu_visible(&self, _: &App) -> bool {
        !self.local
    }
    fn toolbar_buttons(&mut self, _: &mut Window, cx: &mut Context<Self>) -> Option<Vec<Button>> {
        if !self.local {
            return None;
        }
        let workspace = self.workspace.clone();
        let selection = self.selection(cx);
        let id = self.item.item_id();
        let enabled = workspace
            .upgrade()
            .is_some_and(|surface| surface.read(cx).terminal_creation_allowed());
        Some(vec![
            Button::new("local-terminal-new")
                .disabled(!enabled)
                .icon(nocterm_ui::IconName::Plus)
                .tooltip(
                    if self.source.context() == nocterm_project_model::ContextId::General {
                        "New Local Terminal"
                    } else {
                        "New Workspace Terminal"
                    },
                )
                .on_click(move |_, window, cx| {
                    cx.stop_propagation();
                    let _ = workspace.update(cx, |workspace, cx| {
                        if selection.is_some_and(|selection| {
                            workspace.accepts_context_selection(selection, cx)
                        }) {
                            workspace.new_local_terminal_from(id, window, cx);
                        }
                    });
                }),
        ])
    }
    fn free_header_content(
        &mut self,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement> {
        let selection = self.selection(cx);
        self.local.then(|| {
            let origin = self.header_click_origin.clone();
            let id = self.item.item_id();
            div()
                .id("local-terminal-free-header")
                .test_support()
                .size_full()
                .on_mouse_down(gpui_kit::MouseButton::Left, {
                    let origin = origin.clone();
                    move |event, _, _| {
                        if event.click_count == 1 {
                            origin.set(Some(id));
                        }
                    }
                })
                .on_mouse_down_out({
                    let origin = origin.clone();
                    move |event, _, _| {
                        if event.button == gpui_kit::MouseButton::Left
                            && event.click_count == 1
                            && origin.get() == Some(id)
                        {
                            origin.set(None);
                        }
                    }
                })
                .on_click({
                    let workspace = self.workspace.clone();
                    move |event: &gpui_kit::ClickEvent, window: &mut Window, cx: &mut App| {
                        // Closing a tab can expose blank space under the next
                        // click. Both presses must start in this free region.
                        if event.click_count() == 2 && origin.get() == Some(id) {
                            origin.set(None);
                            cx.stop_propagation();
                            let _ = workspace.update(cx, |workspace, cx| {
                                if selection.is_some_and(|selection| {
                                    workspace.accepts_context_selection(selection, cx)
                                }) {
                                    workspace.new_local_terminal_from(id, window, cx);
                                }
                            });
                        }
                    }
                })
        })
    }
    fn inner_padding(&self, _: &App) -> bool {
        false
    }
    fn tab_accent(&self, cx: &App) -> Option<Hsla> {
        let color = GROUP_COLORS[self.group_color? % GROUP_PALETTE];
        Some(color(cx.theme()))
    }
    #[expect(clippy::too_many_lines, reason = "predates the limit")]
    fn title(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let icon_color = match self.item.tab_state(cx) {
            TabState::Idle => cx.theme().muted_foreground,
            TabState::Busy => cx.theme().info,
            TabState::Attention => cx.theme().warning,
            TabState::Ended => cx.theme().danger,
        };
        let workspace = self.workspace.clone();
        let menu_workspace = workspace.clone();
        let selection = self.selection(cx);
        let source = self.source;
        let id = self.item.item_id();
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
                        cx.listener(move |this, event: &gpui_kit::ClickEvent, window, cx| {
                            if event.click_count() == 2
                                && selection.is_some_and(|selection| {
                                    this.workspace.upgrade().is_some_and(|workspace| {
                                        workspace.read(cx).accepts_context_selection(selection, cx)
                                    })
                                })
                            {
                                cx.stop_propagation();
                                if let Some(selection) = selection {
                                    this.start_alias(selection, window, cx);
                                }
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
                            if selection.is_some_and(|selection| {
                                this.accepts_context_selection(selection, cx)
                            }) {
                                this.close_item_by_id(id, window, cx);
                            }
                        });
                    }),
            )
            .context_menu(move |mut menu, window, cx| {
                use crate::workspace::TabCloseScope;
                let selection = menu_workspace.upgrade().and_then(|workspace| {
                    let workspace = workspace.read(cx);
                    (workspace.context_source() == source)
                        .then(|| workspace.context_selection_receipt())
                });
                for (label, scope) in [
                    ("Close Tab", TabCloseScope::Current),
                    ("Close Other Tabs in Pane", TabCloseScope::Others),
                    ("Close Tabs to the Left", TabCloseScope::Left),
                    ("Close Tabs to the Right", TabCloseScope::Right),
                    ("Close All Tabs", TabCloseScope::All),
                ] {
                    let disabled = menu_workspace.upgrade().is_none_or(|workspace| {
                        workspace
                            .read(cx)
                            .content()
                            .tabs_to_close(id, scope, cx)
                            .is_empty()
                    });
                    let workspace = menu_workspace.clone();
                    menu = menu.item(PopupMenuItem::new(label).disabled(disabled).on_click(
                        move |_, window, cx| {
                            let _ = workspace.update(cx, |workspace, cx| {
                                if selection.is_some_and(|selection| {
                                    workspace.accepts_context_selection(selection, cx)
                                }) {
                                    workspace.close_tabs(id, scope, window, cx);
                                }
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
                    let disabled = workspace.upgrade().is_none_or(|workspace| {
                        !workspace.read(cx).content().can_split_item(id, cx)
                    });
                    menu = menu.item(PopupMenuItem::new(label).disabled(disabled).on_click(
                        move |_, window, cx| {
                            let _ = workspace.update(cx, |workspace, cx| {
                                if selection.is_some_and(|selection| {
                                    workspace.accepts_context_selection(selection, cx)
                                }) {
                                    workspace.split_item(id, placement, window, cx);
                                }
                            });
                        },
                    ));
                }
                group_menu(menu.separator(), id, &menu_workspace, selection, window, cx)
                    .separator()
                    .menu("Settings", Box::new(crate::OpenSettings))
            })
    }
}
/// What the tab menu offers for groups of tabs.
fn group_menu(
    mut menu: PopupMenu,
    id: EntityId,
    workspace: &WeakEntity<TabSurface>,
    selection: Option<ContextSelectionReceipt>,
    window: &mut Window,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    let Some(this) = workspace.upgrade() else {
        return menu;
    };
    let (grouped, groups) = {
        let this = this.read(cx);
        (
            this.content().is_tab_grouped(id),
            this.content().tab_groups_to_join(id, cx),
        )
    };
    let item =
        |label: &'static str,
         action: fn(&mut TabSurface, EntityId, &mut Window, &mut Context<TabSurface>)| {
            let workspace = workspace.clone();
            PopupMenuItem::new(label).on_click(move |_, window, cx| {
                let _ = workspace.update(cx, |workspace, cx| {
                    if selection
                        .is_some_and(|selection| workspace.accepts_context_selection(selection, cx))
                    {
                        action(workspace, id, window, cx);
                    }
                });
            })
        };
    menu = menu.item(item("Add to New Group", |surface, id, window, cx| {
        surface.content_mut().new_tab_group(id, window, cx);
        cx.notify();
    }));
    if !groups.is_empty() {
        let workspace = workspace.clone();
        menu = menu.submenu("Add to Group", window, cx, move |mut menu, _, _| {
            for (beside, name) in &groups {
                let (workspace, beside) = (workspace.clone(), *beside);
                menu = menu.item(PopupMenuItem::new(name.clone()).on_click(
                    move |_, window, cx| {
                        let _ = workspace.update(cx, |workspace, cx| {
                            if selection.is_some_and(|selection| {
                                workspace.accepts_context_selection(selection, cx)
                            }) {
                                workspace
                                    .content_mut()
                                    .group_tab_with(id, beside, window, cx);
                                cx.notify();
                            }
                        });
                    },
                ));
            }
            menu
        });
    }
    if grouped {
        menu = menu
            .item(item("Remove from Group", |surface, id, window, cx| {
                surface.content_mut().ungroup_tab(id, window, cx);
                cx.notify();
            }))
            .item(item("Ungroup", |surface, id, window, cx| {
                surface.content_mut().dissolve_tab_group(id, window, cx);
                cx.notify();
            }))
            .item(item("Close Group", TabSurface::close_tab_group));
    }
    menu
}
impl Render for DockItem {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .track_focus(&self.focus)
            .child(self.item.view())
    }
}
