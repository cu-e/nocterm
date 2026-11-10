//! The window's frame around the tabs: the title bar, the body's columns,
//! the sidebar and the footer, which hosts the status bar.
//!
//! In the floating layout ([`FloatingCards`]) every column is a card on the
//! canvas, and the title bar and footer sit on the canvas between them. The
//! dock draws its own tab groups as cards, so the content column only frames
//! what it shows when no tab is open.
use gpui_kit::{
    Anchor, AnyElement, AnyView, App, Context, ElementId, Entity, Hsla, MouseButton,
    StyleRefinement, TestSupportExt as _, Window,
    component::{
        ActiveTheme as _, Sizable as _, StyledExt as _, TitleBar,
        button::{Button, ButtonVariants as _},
        floating::FloatingCards,
        h_flex, h_resizable,
        popover::Popover,
        resizable_panel, v_flex,
    },
    div,
    prelude::*,
    px, svg,
};
use nocterm_plugin_model::{Edge, PanelChrome, Side, WidthClass};
use nocterm_ui::IconName;

use super::{
    ContextSelectionReceipt, MenuBuilder, NewTabMenu, Workspace,
    layout::{Body, Column},
    panel_groups::{GroupKey, HostedPanel, PanelGroups},
    shell_layout::ShellLayout,
    surface::SurfaceFrame,
};
use crate::NewTab;
use std::rc::Rc;

mod empty_state;
mod menus;
mod status_bar;
mod toggle_look;
mod toolbars;

#[cfg(any(test, feature = "test-support"))]
pub(crate) use status_bar::ToggleLooks;
pub(super) use status_bar::{VisibilityHook, WidgetFactory, ZoneHook};
pub(super) use toolbars::{
    TOOLBAR_HEIGHT, TOOLBAR_WIDTH, ToolbarChoice, TopToolbars, sync as sync_toolbars,
};

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
    shell: &'a ShellLayout,
}
impl<'a> Regions<'a> {
    pub(super) fn new(
        tabs: SurfaceFrame,
        selection: ContextSelectionReceipt,
        groups: &'a PanelGroups,
        directory: Option<Rc<dyn crate::ConnectionDirectory>>,
        shell: &'a ShellLayout,
    ) -> Self {
        Self {
            tabs,
            selection,
            groups,
            directory,
            shell,
        }
    }
}

pub(super) struct Chrome {
    status: status_bar::StatusBar,
    toolbars: toolbars::Toolbars,
    menus: menus::WindowMenus,
    new_tab: NewTabControl,
    environment: Option<AnyView>,
}

impl Chrome {
    pub(super) fn new(menu_focus: gpui_kit::FocusHandle, top: Rc<TopToolbars>) -> Self {
        Self {
            status: status_bar::StatusBar::new(),
            toolbars: toolbars::Toolbars::new(top),
            menus: menus::WindowMenus::new(menu_focus),
            new_tab: NewTabControl {
                entry: None,
                open: false,
            },
            environment: None,
        }
    }
    pub(super) fn install_environment(&mut self, view: AnyView) {
        self.environment = Some(view);
    }
    pub(super) fn status_bar(&self) -> &status_bar::StatusBar {
        &self.status
    }
    pub(super) fn status_bar_mut(&mut self) -> &mut status_bar::StatusBar {
        &mut self.status
    }
    pub(super) fn toolbars(&self) -> &toolbars::Toolbars {
        &self.toolbars
    }
    pub(super) fn toolbars_mut(&mut self) -> &mut toolbars::Toolbars {
        &mut self.toolbars
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

/// A toolbar strip as a card of the strip's own size: a card fills what holds it, so without a
/// slot of that size a strip along a side or the bottom would cover the tabs.
fn toolbar_tile(cards: Option<FloatingCards>, bar: AnyElement, horizontal: bool) -> AnyElement {
    let Some(cards) = cards else {
        return bar;
    };
    let inset = cards.margin() * 2.;
    let slot = div().flex_none();
    let slot = if horizontal {
        slot.w_full().h(px(TOOLBAR_HEIGHT) + inset)
    } else {
        slot.h_full().w(px(TOOLBAR_WIDTH) + inset)
    };
    slot.child(cards.card(bar)).into_any_element()
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

    /// One column per shown panel group and the tabs, in the user's order.
    fn render_columns(
        &self,
        body_state: &mut Body,
        regions: &Regions<'_>,
        cards: Option<FloatingCards>,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) -> AnyElement {
        let widths = body_state.widths(window, cx);
        let arrangement = body_state.arrangement(regions.groups);
        let state = body_state.state(&arrangement, cx);
        let mut body = h_resizable("workspace-body").with_state(&state);
        for column in arrangement.columns() {
            body = body.child(match *column {
                Column::Group { group, class, side } => {
                    let width = widths.of(class);
                    resizable_panel()
                        // Only the navigation column has always been fixed in the group.
                        .when(class == WidthClass::Navigation, |panel| panel.flex_none())
                        .map(|panel| {
                            if arrangement.expanded() == Some(group) {
                                panel.size_range(px(0.)..gpui_kit::Pixels::MAX)
                            } else {
                                panel.size(width.size).size_range(width.range.clone())
                            }
                        })
                        .child(self.render_group(regions.groups, group, side, cards, cx))
                }
                Column::Content => resizable_panel().child(self.render_content(regions, cards, cx)),
            });
        }
        body.into_any_element()
    }

    fn render_group(
        &self,
        groups: &PanelGroups,
        group: GroupKey,
        side: Side,
        cards: Option<FloatingCards>,
        cx: &mut Context<Workspace>,
    ) -> AnyElement {
        let panel = groups.selected(group);
        let column = ElementId::from("panel-group-column");
        let column: ElementId = (column, groups.id(group).to_string()).into();
        match panel.map_or(PanelChrome::Titled, HostedPanel::chrome) {
            PanelChrome::Titled => tile(
                cards,
                self.render_titled_group(column, panel, side == Side::Right, cx),
            ),
            PanelChrome::OwnHeader => tile(cards, self.render_own_header_group(column, panel, cx)),
        }
    }

    /// A column whose panel draws its own header.
    fn render_own_header_group(
        &self,
        column: ElementId,
        panel: Option<&HostedPanel>,
        cx: &mut Context<Workspace>,
    ) -> impl IntoElement {
        Workspace::forward_commands(div(), cx)
            .id(column)
            .test_support()
            .size_full()
            .min_w_0()
            .when_some(panel.map(HostedPanel::view), |body, view| {
                body.child(cached(view))
            })
    }

    /// The open tabs, or an invitation to open one, with the toolbars along their edges.
    fn render_content(
        &self,
        regions: &Regions<'_>,
        cards: Option<FloatingCards>,
        cx: &mut Context<Workspace>,
    ) -> AnyElement {
        let anchor = regions
            .tabs
            .visible()
            .then(|| regions.tabs.only_group())
            .flatten();
        if self.toolbars.top().set_anchor(anchor) && !self.toolbars.top().is_empty() {
            // The dock caches its tab groups: the one that gains or loses the row draws again.
            let surface = regions.tabs.view().entity_id();
            App::notify(cx, surface);
        }
        let content = if regions.tabs.visible() {
            // The dock frames each of its tab groups itself.
            div()
                .id("workspace-content")
                .test_support()
                .size_full()
                // Cached: sidebar and chrome redraws reuse the tabs unless they changed.
                .child(cached(regions.tabs.view()))
                .into_any_element()
        } else {
            tile(
                cards,
                self.render_empty_state(regions.directory.clone(), regions.selection, cx),
            )
        };
        self.frame_with_toolbars(content, regions, anchor.is_none(), cards, cx)
    }

    /// `content` with the toolbars of the bottom and side edges around it, and those of the top
    /// edge above it when no tab group draws them under its tab bar.
    fn frame_with_toolbars(
        &self,
        content: AnyElement,
        regions: &Regions<'_>,
        top_above: bool,
        cards: Option<FloatingCards>,
        cx: &mut Context<Workspace>,
    ) -> AnyElement {
        let layout = regions.shell.current();
        let top = top_above.then(|| self.toolbars.top().row(cx)).flatten();
        let left = self.toolbars.render_edge(layout, Edge::Left, cx);
        let right = self.toolbars.render_edge(layout, Edge::Right, cx);
        let bottom = self.toolbars.render_edge(layout, Edge::Bottom, cx);
        if top.is_none() && left.is_none() && right.is_none() && bottom.is_none() {
            return content;
        }
        v_flex()
            .id("workspace-tab-area")
            .test_support()
            .size_full()
            .min_w_0()
            .min_h_0()
            .children(top.map(|bar| toolbar_tile(cards, bar, true)))
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .children(left.map(|bar| toolbar_tile(cards, bar, false)))
                    .child(div().flex_1().min_w_0().h_full().child(content))
                    .children(right.map(|bar| toolbar_tile(cards, bar, false))),
            )
            .children(bottom.map(|bar| toolbar_tile(cards, bar, true)))
            .into_any_element()
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
                    .child(
                        svg()
                            .path(nocterm_ui::APP_MARK)
                            .id("workspace-app-mark")
                            .test_support()
                            .flex_none()
                            .w(px(18.))
                            .h(px(9.))
                            .text_color(cx.theme().foreground),
                    )
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
                    .when_some(self.environment.as_ref(), |bar, environment| {
                        bar.child(
                            div()
                                .id("workspace-environment")
                                .test_support()
                                .flex_none()
                                .h_full()
                                .child(environment.clone()),
                        )
                    })
                    .child(div().flex_1())
                    .child(self.new_tab.render(selection, cx)),
            )
    }

    /// A column under the workspace's own upper-case title.
    fn render_titled_group(
        &self,
        column: ElementId,
        panel: Option<&HostedPanel>,
        on_right: bool,
        cx: &mut Context<Workspace>,
    ) -> impl IntoElement {
        let sidebar = Workspace::forward_commands(v_flex(), cx);
        let theme = cx.theme();
        let floating = FloatingCards::get(cx).is_some();

        sidebar
            .id(column)
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
                let provenance = panel.provenance(cx).map(|provenance| {
                    provenance_chip(provenance, theme.border, theme.muted_foreground)
                });
                sidebar
                    .child(
                        h_flex()
                            .gap_1p5()
                            .px_3()
                            .pt_2()
                            .pb_1()
                            .text_xs()
                            .font_semibold()
                            .text_color(theme.muted_foreground)
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .child(panel.title(cx).to_uppercase()),
                            )
                            .children(provenance),
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
        let inputs = self.bar_inputs(regions, cx);
        let menu = status_bar::BarMenu::new(
            inputs.pointed().clone(),
            inputs.entries(),
            regions.shell.set(),
            regions.shell.choices(),
            cx.weak_entity(),
        )
        .with_toolbars(
            self.toolbars
                .choices(regions.shell.set(), regions.shell.current()),
        );
        status_bar::footer(inputs, menu, cx)
    }

    /// What the status bar draws for the window as it is now.
    pub(super) fn bar_inputs(&self, regions: &Regions<'_>, cx: &App) -> status_bar::BarInputs {
        status_bar::BarInputs::new(regions, &self.status, cx)
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

/// The chip that says who supplied a panel the application did not, written by the shell beside
/// the panel's title, where the panel cannot draw.
fn provenance_chip(
    provenance: crate::Provenance,
    border: gpui_kit::Hsla,
    text: gpui_kit::Hsla,
) -> impl IntoElement {
    let detail = provenance.detail;
    div()
        .id("panel-provenance")
        .debug_selector(|| "panel-provenance".to_string())
        .flex_none()
        .px_1()
        .rounded_sm()
        .border_1()
        .border_color(border)
        .text_color(text)
        .font_weight(gpui_kit::FontWeight::MEDIUM)
        .child(provenance.chip)
        .tooltip(move |window, cx| {
            gpui_kit::component::tooltip::Tooltip::new(detail.clone()).build(window, cx)
        })
}
