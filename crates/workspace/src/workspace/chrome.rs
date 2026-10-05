//! The window's frame around the tabs: the title bar, the body's columns,
//! the sidebar and the footer.
use gpui_kit::{
    Action as _, Anchor, AnyElement, AnyView, Context, Entity, MouseButton, StyleRefinement,
    TestSupportExt as _, Window,
    component::{
        ActiveTheme as _, Selectable as _, Sizable as _, StyledExt as _, TitleBar,
        button::{Button, ButtonVariants as _},
        h_flex, h_resizable,
        popover::Popover,
        resizable_panel, v_flex,
    },
    div,
    prelude::*,
};
use nocterm_ui::IconName;

use super::{
    NewTabMenu, Workspace,
    layout::{BodyWidths, Column},
};
use crate::NewTab;

/// `view`, drawn again only when it notifies or the window is refreshed.
/// Whatever redraws the workspace (each keystroke in a terminal does) would
/// otherwise rebuild every panel beside it.
fn cached(view: AnyView) -> impl IntoElement {
    view.cached(StyleRefinement::default().size_full())
}

/// Which end of the footer a feature's status view sits at.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum StatusSide {
    Leading,
    Trailing,
}

impl Workspace {
    /// A feature-owned compact status view, visible even while the sidebar is hidden.
    pub fn add_status_view<V: Render>(&mut self, view: Entity<V>, cx: &mut Context<Self>) {
        self.status_views.push((StatusSide::Trailing, view.into()));
        cx.notify();
    }

    /// A feature-owned status view at the start of the footer, after the
    /// sidebar's panel buttons: room for what describes the active host.
    pub fn add_leading_status_view<V: Render>(&mut self, view: Entity<V>, cx: &mut Context<Self>) {
        self.status_views.push((StatusSide::Leading, view.into()));
        cx.notify();
    }

    fn status_views(&self, side: StatusSide) -> impl Iterator<Item = gpui_kit::AnyView> + '_ {
        self.status_views
            .iter()
            .filter(move |(at, _)| *at == side)
            .map(|(_, view)| view.clone())
    }

    /// The sidebar, the tabs and the side panel, in the user's order.
    pub(super) fn render_body(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if self.right_panel_maximized {
            return div()
                .id("workspace-right-panel")
                .test_support()
                .size_full()
                .when_some(self.right_panel.as_ref(), |body, panel| {
                    body.child(cached(panel.view()))
                })
                .into_any_element();
        }
        let widths = BodyWidths::new(window, cx);
        let arrangement = self.arrangement(cx);
        let state = self.body.state(arrangement, cx);
        let mut body = h_resizable("workspace-body").with_state(&state);
        for column in arrangement.columns() {
            body = body.child(match column {
                Column::Sidebar => resizable_panel()
                    .size(widths.sidebar)
                    .size_range(widths.sidebar_range.clone())
                    .child(self.render_sidebar(arrangement.swapped, cx)),
                Column::Content => resizable_panel().child(self.render_content(cx)),
                Column::SidePanel => resizable_panel()
                    .size(widths.right_panel)
                    .size_range(widths.right_panel_range.clone())
                    .child(
                        div()
                            .id("workspace-right-panel")
                            .test_support()
                            .size_full()
                            .when_some(self.right_panel.as_ref(), |body, panel| {
                                body.child(cached(panel.view()))
                            }),
                    ),
            });
        }
        body.into_any_element()
    }

    pub(super) fn render_title_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        TitleBar::new().child(
            h_flex()
                .size_full()
                .min_w_0()
                .gap_1()
                .pr_2()
                .when_some(self.app_menu_bar.as_ref(), |bar, menu| {
                    bar.child(
                        div()
                            .id("workspace-app-menu")
                            .track_focus(&self.menu_focus)
                            .h_full()
                            .min_w_0()
                            .capture_any_mouse_down(cx.listener(
                                |this, event: &gpui_kit::MouseDownEvent, window, cx| {
                                    if event.button == MouseButton::Left {
                                        this.prepare_menu(window, cx);
                                    }
                                },
                            ))
                            .capture_key_down(cx.listener(
                                |this, event: &gpui_kit::KeyDownEvent, window, cx| {
                                    match event.keystroke.key.as_str() {
                                        "enter" | "space" => this.prepare_menu(window, cx),
                                        _ => {}
                                    }
                                },
                            ))
                            .child(menu.clone()),
                    )
                })
                .child(div().flex_1())
                .child(self.render_new_tab_button(cx)),
        )
    }

    fn render_new_tab_button(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let button = Button::new("new-tab")
            .ghost()
            .small()
            .icon(IconName::Plus)
            .tooltip("New Tab");

        match &self.new_tab_menu {
            None => button
                .on_click(|_, window, cx| window.dispatch_action(NewTab.boxed_clone(), cx))
                .into_any_element(),
            Some(NewTabMenu { view, focus }) => {
                let menu = view.clone();
                let workspace = cx.entity().downgrade();
                Popover::new("new-tab-menu")
                    .anchor(Anchor::TopRight)
                    .trigger(button)
                    .track_focus(focus)
                    .open(self.new_tab_menu_open)
                    .on_open_change(move |open, _, cx| {
                        let _ = workspace.update(cx, |this, cx| this.show_new_tab_menu(*open, cx));
                    })
                    .content(move |_, _, _| menu.clone())
                    .into_any_element()
            }
        }
    }

    fn render_sidebar(&self, on_right: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let panel = self.panels.get(self.active_panel);

        v_flex()
            .size_full()
            .bg(theme.sidebar)
            .text_color(theme.sidebar_foreground)
            .map(|sidebar| {
                if on_right {
                    sidebar.border_l_1()
                } else {
                    sidebar.border_r_1()
                }
            })
            .border_color(theme.sidebar_border)
            .when_some(panel, |sidebar, panel| {
                sidebar
                    .child(
                        div()
                            .px_3()
                            .pt_2()
                            .pb_1()
                            .text_xs()
                            .font_semibold()
                            .text_color(theme.muted_foreground)
                            .child(panel.title(cx).to_uppercase()),
                    )
                    .child(div().flex_1().min_h_0().child(cached(panel.view())))
            })
            .when(panel.is_none(), |sidebar| sidebar.child(div().flex_1()))
    }

    pub(super) fn render_footer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        div().id("workspace-footer").w_full().flex_shrink_0().child(
            h_flex()
                .gap_1()
                .px_2()
                .py_1()
                .border_t_1()
                .border_color(theme.sidebar_border)
                .children(self.panels.iter().enumerate().map(|(ix, panel)| {
                    Button::new(("sidebar-panel", ix))
                        .ghost()
                        .small()
                        .icon(panel.icon(cx))
                        .tooltip(panel.title(cx))
                        .when_some(panel.badge(cx), Button::label)
                        .selected(self.sidebar_open && ix == self.active_panel)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            // The shown panel's button hides the sidebar.
                            if this.sidebar_open && this.active_panel == ix {
                                this.toggle_sidebar(window, cx);
                            } else {
                                this.activate_panel(ix, window, cx);
                            }
                        }))
                }))
                .children(self.status_views(StatusSide::Leading))
                .child(div().flex_1())
                .child(nocterm_ui::notice::NoticeBar::new())
                .children(self.status_views(StatusSide::Trailing))
                .child(
                    Button::new("toggle-local-terminal")
                        .ghost()
                        .small()
                        .icon(IconName::SquareTerminal)
                        .tooltip("Local Terminal")
                        .selected(self.local_terminal_is_visible(cx))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.toggle_local_terminal(window, cx)
                        })),
                )
                .when(self.right_panel_is_available(), |footer| {
                    footer.child(
                        Button::new("toggle-right-panel")
                            .ghost()
                            .small()
                            .icon(if self.right_panel_attention {
                                IconName::ShieldCheck
                            } else {
                                IconName::PanelRight
                            })
                            .tooltip(if self.right_panel_attention {
                                "AI Agents: permission required"
                            } else {
                                "AI Agents"
                            })
                            .selected(self.right_panel_open)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.toggle_right_panel(window, cx)
                            })),
                    )
                }),
        )
    }
}
