//! The window's frame around the tabs: the title bar, the body's columns,
//! the sidebar and the footer.
//!
//! In the floating layout ([`FloatingCards`]) every column is a card on the
//! canvas, and the title bar and footer sit on the canvas between them. The
//! dock draws its own tab groups as cards, so the content column only frames
//! what it shows when no tab is open.
use gpui_kit::{
    Action as _, Anchor, AnyElement, AnyView, App, Context, Entity, Hsla, MouseButton,
    StyleRefinement, TestSupportExt as _, Window,
    component::{
        ActiveTheme as _, Selectable as _, Sizable as _, StyledExt as _, TitleBar,
        button::{Button, ButtonVariants as _},
        floating::FloatingCards,
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

mod empty_state;

/// `view`, drawn again only when it notifies or the window is refreshed.
/// Whatever redraws the workspace (each keystroke in a terminal does) would
/// otherwise rebuild every panel beside it.
fn cached(view: AnyView) -> impl IntoElement {
    view.cached(StyleRefinement::default().size_full())
}

/// `content` as a card when the window floats, otherwise as it is.
fn tile(cards: Option<FloatingCards>, content: impl IntoElement) -> AnyElement {
    match cards {
        Some(cards) => cards.card(content).into_any_element(),
        None => content.into_any_element(),
    }
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

    /// What the window is painted with behind its regions: the canvas when
    /// they float, the interface background otherwise.
    pub(super) fn window_background(cx: &App) -> Hsla {
        FloatingCards::get(cx).map_or(cx.theme().background, |cards| cards.canvas)
    }

    /// The body inside the half gap that, with each card's own margin, keeps
    /// floating cards a full gap from the window's edge.
    pub(super) fn render_body(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let cards = FloatingCards::get(cx);
        div()
            .size_full()
            .when_some(cards, |body, cards| body.p(cards.margin()))
            .child(self.render_columns(cards, window, cx))
            .into_any_element()
    }

    /// The sidebar, the tabs and the side panel, in the user's order.
    fn render_columns(
        &mut self,
        cards: Option<FloatingCards>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if self.right_panel_maximized {
            return tile(
                cards,
                Self::forward_commands(div(), cx)
                    .id("workspace-right-panel")
                    .test_support()
                    .size_full()
                    .when_some(self.right_panel.as_ref(), |body, panel| {
                        body.child(cached(panel.view()))
                    }),
            );
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
                    .child(tile(cards, self.render_sidebar(arrangement.swapped, cx))),
                Column::Content => resizable_panel().child(self.render_content(cards, cx)),
                Column::SidePanel => resizable_panel()
                    .size(widths.right_panel)
                    .size_range(widths.right_panel_range.clone())
                    .child(tile(
                        cards,
                        Self::forward_commands(div(), cx)
                            .id("workspace-right-panel")
                            .test_support()
                            .size_full()
                            .when_some(self.right_panel.as_ref(), |body, panel| {
                                body.child(cached(panel.view()))
                            }),
                    )),
            });
        }
        body.into_any_element()
    }

    /// The open tabs, or an invitation to open one.
    fn render_content(&self, cards: Option<FloatingCards>, cx: &mut Context<Self>) -> AnyElement {
        if self
            .items
            .iter()
            .any(|open| self.item_visible(open.handle.item_id(), cx))
        {
            // The dock frames each of its tab groups itself.
            return div()
                .size_full()
                .child(self.dock.clone())
                .into_any_element();
        }
        tile(cards, self.render_empty_state(cx))
    }

    pub(super) fn render_title_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let cards = FloatingCards::get(cx);
        TitleBar::new()
            // On the canvas, the title bar is part of the backdrop.
            .when_some(cards, |bar, cards| bar.bg(cards.canvas).border_b_0())
            .child(
                Self::forward_commands(h_flex(), cx)
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
        let sidebar = Self::forward_commands(v_flex(), cx);
        let theme = cx.theme();
        let panel = self.panels.get(self.active_panel);
        let floating = FloatingCards::get(cx).is_some();

        sidebar
            .size_full()
            .text_color(theme.sidebar_foreground)
            // A card brings its own fill and outline.
            .when(!floating, |sidebar| {
                sidebar
                    .bg(theme.sidebar)
                    .map(|sidebar| {
                        if on_right {
                            sidebar.border_l_1()
                        } else {
                            sidebar.border_r_1()
                        }
                    })
                    .border_color(theme.sidebar_border)
            })
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
        let footer = Self::forward_commands(div(), cx);
        let theme = cx.theme();
        let floating = FloatingCards::get(cx).is_some();
        footer
            .id("workspace-footer")
            .w_full()
            .flex_shrink_0()
            .child(
                h_flex()
                    .gap_1()
                    .px_2()
                    .py_1()
                    // On the canvas the gap above already separates the footer.
                    .when(!floating, |footer| {
                        footer.border_t_1().border_color(theme.sidebar_border)
                    })
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
