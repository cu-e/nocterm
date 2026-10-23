//! The window's frame around the tabs: the title bar, the body's columns,
//! the sidebar and the footer.
//!
//! In the floating layout ([`FloatingCards`]) every column is a card on the
//! canvas, and the title bar and footer sit on the canvas between them. The
//! dock draws its own tab groups as cards, so the content column only frames
//! what it shows when no tab is open.
use gpui_kit::{
    Anchor, AnyElement, AnyView, App, Context, ElementId, Entity, Hsla, MouseButton,
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
    px,
};
use nocterm_ui::IconName;

use super::{
    ContextSelectionReceipt, MenuBuilder, NewTabMenu, Workspace,
    layout::{Body, Column, PanelPresentation},
    panel_groups::{GroupKey, PanelGroups},
    surface::SurfaceFrame,
};
use crate::NewTab;
use std::rc::Rc;

mod empty_state;
mod menus;
mod toggle_look;

pub(super) use toggle_look::FooterLooks;

struct NewTabControl {
    entry: Option<NewTabMenu>,
    open: bool,
}

/// Ephemeral immutable regions; Chrome cannot mutate another state owner.
pub(super) struct Regions<'a> {
    tabs: SurfaceFrame,
    selection: ContextSelectionReceipt,
    groups: &'a PanelGroups,
    directory: Option<Rc<dyn crate::ConnectionDirectory>>,
}
impl<'a> Regions<'a> {
    pub(super) fn new(
        tabs: SurfaceFrame,
        selection: ContextSelectionReceipt,
        groups: &'a PanelGroups,
        directory: Option<Rc<dyn crate::ConnectionDirectory>>,
    ) -> Self {
        Self {
            tabs,
            selection,
            groups,
            directory,
        }
    }
}

pub(super) struct Chrome {
    status: Vec<(StatusSide, AnyView)>,
    menus: menus::WindowMenus,
    new_tab: NewTabControl,
}

impl Chrome {
    pub(super) fn new(menu_focus: gpui_kit::FocusHandle) -> Self {
        Self {
            status: Vec::new(),
            menus: menus::WindowMenus::new(menu_focus),
            new_tab: NewTabControl {
                entry: None,
                open: false,
            },
        }
    }
    pub(super) fn add_status(&mut self, side: StatusSide, view: AnyView) {
        self.status.push((side, view));
    }
    pub(super) fn status_views(&self, side: StatusSide) -> impl Iterator<Item = AnyView> + '_ {
        self.status
            .iter()
            .filter(move |(at, _)| *at == side)
            .map(|(_, view)| view.clone())
    }
    pub(super) fn install_new_tab(&mut self, view: AnyView, focus: gpui_kit::FocusHandle) {
        self.new_tab.install(view, focus);
    }
    pub(super) fn new_tab_open(&self) -> bool {
        self.new_tab.open
    }
    pub(super) fn show_new_tab(&mut self, open: bool) -> bool {
        self.new_tab.show(open)
    }
    pub(super) fn install_menu_builder(&mut self, builder: MenuBuilder) {
        self.menus.install(builder);
    }
    pub(super) fn menu_builder(&self) -> Option<MenuBuilder> {
        self.menus.builder()
    }
    pub(super) fn stale_menu_selection(
        &self,
        selection: ContextSelectionReceipt,
        window: &Window,
        cx: &App,
    ) -> bool {
        self.menus.stale_selection(selection, window, cx)
    }
    pub(super) fn command_target(
        &self,
        window: &Window,
        last: Option<gpui_kit::EntityId>,
        cx: &App,
    ) -> Option<gpui_kit::EntityId> {
        self.menus.command_target(window, last, cx)
    }
    pub(super) fn menu_focus(&self) -> &gpui_kit::FocusHandle {
        self.menus.focus()
    }
    pub(super) fn menu_bar(&self) -> Option<Entity<gpui_kit::component::menu::AppMenuBar>> {
        self.menus.bar()
    }
    pub(super) fn refresh_menus(
        &mut self,
        menus: Vec<gpui_kit::OwnedMenu>,
        target: Option<gpui_kit::EntityId>,
        selection: ContextSelectionReceipt,
        cx: &mut App,
    ) {
        self.menus.refresh(menus, target, selection, cx);
    }
    pub(super) fn prepare_closed_menu(&mut self, cx: &App) -> bool {
        self.menus.clear_closed_target(cx)
    }
}

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

impl Chrome {
    /// What the window is painted with behind its regions: the canvas when
    /// they float, the interface background otherwise.
    pub(super) fn window_background(cx: &App) -> Hsla {
        FloatingCards::get(cx).map_or(cx.theme().background, |cards| cards.canvas)
    }

    /// The body inside the half gap that, with each card's own margin, keeps
    /// floating cards a full gap from the window's edge.
    pub(super) fn render_body(
        &self,
        body: &mut Body,
        regions: &Regions<'_>,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) -> AnyElement {
        let cards = FloatingCards::get(cx);
        div()
            .size_full()
            .when_some(cards, |body, cards| body.p(cards.margin()))
            .child(self.render_columns(body, regions, cards, window, cx))
            .into_any_element()
    }

    /// The sidebar, the tabs and the side panel, in the user's order.
    fn render_columns(
        &self,
        body_state: &mut Body,
        regions: &Regions<'_>,
        cards: Option<FloatingCards>,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) -> AnyElement {
        let widths = body_state.widths(window, cx);
        let presentation = PanelPresentation::new(
            regions.groups.is_shown(GroupKey::Assistant),
            regions.groups.is_expanded(GroupKey::Assistant),
        );
        let arrangement = body_state.arrangement(
            regions.groups.is_shown(GroupKey::Navigation),
            presentation,
            cx,
        );
        let state = body_state.state(arrangement, cx);
        let mut body = h_resizable("workspace-body").with_state(&state);
        for column in arrangement.columns() {
            body = body.child(match column {
                Column::Sidebar => resizable_panel()
                    .flex_none()
                    .size(widths.sidebar)
                    .size_range(widths.sidebar_range.clone())
                    .child(tile(
                        cards,
                        self.render_sidebar(regions.groups, arrangement.swapped, cx),
                    )),
                Column::Content => resizable_panel().child(self.render_content(regions, cards, cx)),
                Column::SidePanel => resizable_panel()
                    .map(|panel| match presentation {
                        PanelPresentation::Docked => panel
                            .size(widths.right_panel)
                            .size_range(widths.right_panel_range.clone()),
                        _ => panel.size_range(px(0.)..gpui_kit::Pixels::MAX),
                    })
                    .child(self.render_right_panel(regions.groups, cards, cx)),
            });
        }
        body.into_any_element()
    }

    fn render_right_panel(
        &self,
        groups: &PanelGroups,
        cards: Option<FloatingCards>,
        cx: &mut Context<Workspace>,
    ) -> AnyElement {
        tile(
            cards,
            Workspace::forward_commands(div(), cx)
                .id("workspace-right-panel")
                .test_support()
                .size_full()
                .min_w_0()
                .when_some(
                    groups
                        .selected(GroupKey::Assistant)
                        .map(|panel| panel.view()),
                    |body, view| body.child(cached(view)),
                ),
        )
    }

    /// The open tabs, or an invitation to open one.
    fn render_content(
        &self,
        regions: &Regions<'_>,
        cards: Option<FloatingCards>,
        cx: &mut Context<Workspace>,
    ) -> AnyElement {
        if regions.tabs.visible() {
            // The dock frames each of its tab groups itself.
            return div()
                .id("workspace-content")
                .test_support()
                .size_full()
                .child(regions.tabs.view())
                .into_any_element();
        }
        tile(
            cards,
            self.render_empty_state(regions.directory.clone(), regions.selection, cx),
        )
    }

    pub(super) fn render_title_bar(
        &self,
        selection: ContextSelectionReceipt,
        cx: &mut Context<Workspace>,
    ) -> impl IntoElement {
        let cards = FloatingCards::get(cx);
        TitleBar::new()
            // On the canvas, the title bar is part of the backdrop.
            .when_some(cards, |bar, cards| bar.bg(cards.canvas).border_b_0())
            .child(
                Workspace::forward_commands(h_flex(), cx)
                    .size_full()
                    .min_w_0()
                    .gap_1()
                    .pr_2()
                    .when_some(self.menu_bar().as_ref(), |bar, menu| {
                        bar.child(
                            div()
                                .id("workspace-app-menu")
                                .track_focus(self.menu_focus())
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
                    .child(self.new_tab.render(selection, cx)),
            )
    }

    fn render_sidebar(
        &self,
        groups: &PanelGroups,
        on_right: bool,
        cx: &mut Context<Workspace>,
    ) -> impl IntoElement {
        let sidebar = Workspace::forward_commands(v_flex(), cx);
        let theme = cx.theme();
        let panel = groups.selected(GroupKey::Navigation);
        let floating = FloatingCards::get(cx).is_some();

        sidebar
            .id((ElementId::from("panel-group-column"), "navigation"))
            .test_support()
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

    pub(super) fn render_footer(
        &self,
        regions: &Regions<'_>,
        cx: &mut Context<Workspace>,
    ) -> impl IntoElement {
        let footer = Workspace::forward_commands(div(), cx);
        let selection = regions.selection;
        let theme = cx.theme();
        let floating = FloatingCards::get(cx).is_some();
        let FooterLooks {
            navigation,
            local_terminal,
            side_panel,
        } = FooterLooks::of(regions.groups, cx);
        footer
            .id("workspace-footer")
            .test_support()
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
                    .children(
                        regions
                            .groups
                            .members(GroupKey::Navigation)
                            .zip(navigation)
                            .enumerate()
                            .map(|(ix, (panel, look))| {
                                Button::new(("sidebar-panel", ix))
                                    .ghost()
                                    .small()
                                    .icon(look.icon)
                                    .tooltip(look.tooltip)
                                    .when_some(panel.badge(cx), Button::label)
                                    .selected(regions.groups.is_showing(GroupKey::Navigation, ix))
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        // The shown panel's button hides the sidebar.
                                        this.toggle_sidebar_panel(ix, window, cx);
                                    }))
                            }),
                    )
                    .children(self.status_views(StatusSide::Leading))
                    .child(div().flex_1())
                    .child(nocterm_ui::notice::NoticeBar::new())
                    .children(self.status_views(StatusSide::Trailing))
                    .child(
                        Button::new("toggle-local-terminal")
                            .ghost()
                            .small()
                            .icon(local_terminal.icon)
                            .tooltip(local_terminal.tooltip)
                            .selected(regions.tabs.local_visible())
                            .on_click(cx.listener(move |this, _, window, cx| {
                                if this.accepts_context_selection(selection, cx) {
                                    this.toggle_local_terminal(window, cx);
                                }
                            })),
                    )
                    .when_some(side_panel, |footer, side| {
                        footer.child(
                            Button::new("toggle-right-panel")
                                .ghost()
                                .small()
                                .icon(side.icon)
                                .tooltip(side.tooltip)
                                .selected(regions.groups.is_shown(GroupKey::Assistant))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.toggle_right_panel(window, cx)
                                })),
                        )
                    }),
            )
    }
}

impl NewTabControl {
    fn install(&mut self, view: AnyView, focus: gpui_kit::FocusHandle) {
        self.entry = Some(NewTabMenu { view, focus });
    }
    fn show(&mut self, open: bool) -> bool {
        let changed = self.open != open;
        self.open = open;
        changed
    }
    fn render(
        &self,
        selection: ContextSelectionReceipt,
        cx: &mut Context<Workspace>,
    ) -> impl IntoElement {
        let button = Button::new("new-tab")
            .ghost()
            .small()
            .icon(IconName::Plus)
            .tooltip("New Tab");

        match &self.entry {
            None => button
                .on_click(cx.listener(move |this, _, window, cx| {
                    if this.accepts_context_selection(selection, cx) {
                        this.on_new_tab(&NewTab, window, cx);
                    }
                }))
                .into_any_element(),
            Some(NewTabMenu { view, focus }) => {
                let menu = view.clone();
                let workspace = cx.entity().downgrade();
                Popover::new("new-tab-menu")
                    .anchor(Anchor::TopRight)
                    .trigger(button)
                    .track_focus(focus)
                    .open(self.open)
                    .on_open_change(move |open, _, cx| {
                        let _ = workspace.update(cx, |this, cx| {
                            if this.accepts_context_selection(selection, cx) {
                                this.show_new_tab_menu(*open, cx);
                            }
                        });
                    })
                    .content(move |_, _, _| menu.clone())
                    .into_any_element()
            }
        }
    }
}
