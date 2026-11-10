//! Toolbars: the views registered for the edges of the tab area.
//!
//! [`Toolbars`] holds each registered toolbar's factory and creates the view only while the
//! arrangement shows the toolbar along an edge, as the status bar does for its widgets, and again
//! when it turns from a row into a column or back. The top edge is drawn by the outer dock under
//! the tab bar of its only tab group, through the [`TopToolbars`] slot both share; the chrome
//! draws it above the tabs when there is no such group, and the other edges around the tabs. A
//! toolbar that runs inside its plugin's own tabs has no factory: the window lists it and keeps
//! its place, and its tabs draw it.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gpui_kit::{
    AnyElement, AnyView, App, Context, Window,
    component::{ActiveTheme as _, dock::NodeId, h_flex, v_flex},
    div,
    prelude::*,
    px,
};
use nocterm_plugin_model::{ContributionId, Edge, Icon, PluginSet, ToolbarTabs};
use nocterm_window_layout::ResolvedLayout;

use super::status_bar::WidgetFactory;
use crate::Workspace;

/// The height of a toolbar along the top or the bottom edge.
pub(in crate::workspace) const TOOLBAR_HEIGHT: f32 = 32.;
/// The width of a toolbar along the left or the right edge.
pub(in crate::workspace) const TOOLBAR_WIDTH: f32 = 36.;

struct ToolbarEntry {
    id: ContributionId,
    /// `None` for a toolbar its plugin's tabs draw.
    factory: Option<WidgetFactory>,
    /// `None` while the arrangement does not show the toolbar.
    view: Option<AnyView>,
    /// The edge the arrangement gave it when the toolbars were last placed.
    edge: Option<Edge>,
}

/// The registered toolbars, in registration order.
pub(in crate::workspace) struct Toolbars {
    entries: Vec<ToolbarEntry>,
    top: Rc<TopToolbars>,
}

impl Toolbars {
    pub(in crate::workspace) fn new(top: Rc<TopToolbars>) -> Self {
        Self {
            entries: Vec::new(),
            top,
        }
    }

    /// Registers a toolbar; its view comes with [`Toolbars::fill`]. Without a factory the window
    /// lists it but never draws it.
    pub(in crate::workspace) fn add(&mut self, id: ContributionId, factory: Option<WidgetFactory>) {
        self.entries.push(ToolbarEntry {
            id,
            factory,
            view: None,
            edge: None,
        });
    }

    /// Forgets a toolbar, so its plugin's uninstall or replacement leaves no view behind.
    pub(in crate::workspace) fn remove(&mut self, id: &ContributionId) {
        self.entries.retain(|entry| entry.id != *id);
    }

    /// Records the edge the arrangement gives each toolbar and drops the view of one it hides or
    /// turns, so a view is always drawn for its axis. Returns whether any edge changed.
    pub(in crate::workspace) fn place(&mut self, layout: &ResolvedLayout) -> bool {
        let mut changed = false;
        for entry in &mut self.entries {
            let edge = layout.toolbar_edge(&entry.id);
            if edge == entry.edge {
                continue;
            }
            changed = true;
            if edge.map(Edge::is_horizontal) != entry.edge.map(Edge::is_horizontal) {
                entry.view = None;
            }
            entry.edge = edge;
        }
        changed
    }

    /// The toolbars the window draws and shows that have no view yet.
    pub(in crate::workspace) fn pending(&self) -> Vec<(ContributionId, WidgetFactory)> {
        self.entries
            .iter()
            .filter(|entry| entry.view.is_none() && entry.edge.is_some())
            .filter_map(|entry| Some((entry.id.clone(), entry.factory.clone()?)))
            .collect()
    }

    /// Keeps the view a factory created.
    pub(in crate::workspace) fn fill(&mut self, id: &ContributionId, view: AnyView) {
        if let Some(entry) = self.entries.iter_mut().find(|entry| entry.id == *id) {
            entry.view = Some(view);
        }
    }

    /// Hands the top edge's views to the slot the outer dock reads. Returns whether they changed.
    pub(in crate::workspace) fn publish_top(&self, layout: &ResolvedLayout) -> bool {
        self.top.set_views(self.views(layout, Edge::Top))
    }

    /// Draws the toolbars again: an edge caches them, and one may show the workspace's state.
    pub(in crate::workspace) fn redraw(&self, cx: &mut App) {
        for view in self.entries.iter().filter_map(|entry| entry.view.as_ref()) {
            cx.notify(view.entity_id());
        }
    }

    pub(in crate::workspace) fn view(&self, id: &ContributionId) -> Option<&AnyView> {
        self.entries
            .iter()
            .find(|entry| entry.id == *id)
            .and_then(|entry| entry.view.as_ref())
    }

    /// The views along `edge`, in the arrangement's order.
    fn views(&self, layout: &ResolvedLayout, edge: Edge) -> Vec<AnyView> {
        layout
            .toolbars(edge)
            .iter()
            .filter_map(|id| self.view(id).cloned())
            .collect()
    }

    /// The toolbars of this window as the status bar menu offers them: each with its name, icon,
    /// the tabs it runs along and the edge that shows it.
    pub(in crate::workspace) fn choices(
        &self,
        set: &PluginSet,
        layout: &ResolvedLayout,
    ) -> Vec<ToolbarChoice> {
        self.entries
            .iter()
            .filter_map(|entry| {
                let contribution = set.contribution(&entry.id)?;
                let title = match set.plugin_of(&entry.id) {
                    Some(plugin) if contribution.is_user() => {
                        set.user_label(plugin.name(), contribution.title())
                    }
                    _ => contribution.title().to_owned(),
                };
                Some(ToolbarChoice {
                    id: entry.id.clone(),
                    title,
                    icon: contribution.icon(),
                    own_tabs: contribution.tabs() == Some(ToolbarTabs::Own),
                    shown: layout.toolbar_edge(&entry.id),
                })
            })
            .collect()
    }

    /// The edge the arrangement gives a registered toolbar, or `None` while it is hidden.
    pub(in crate::workspace) fn edge(&self, id: &ContributionId) -> Option<Edge> {
        self.entries
            .iter()
            .find(|entry| entry.id == *id)
            .and_then(|entry| entry.edge)
    }

    pub(in crate::workspace) fn top(&self) -> &Rc<TopToolbars> {
        &self.top
    }

    /// The toolbars along a side or the bottom edge, or `None` when the edge shows none.
    pub(in crate::workspace) fn render_edge(
        &self,
        layout: &ResolvedLayout,
        edge: Edge,
        cx: &mut Context<Workspace>,
    ) -> Option<AnyElement> {
        let views = self.views(layout, edge);
        (!views.is_empty()).then(|| edge_bar(edge, views, cx))
    }
}

/// A toolbar the status bar menu shows, hides or moves.
#[derive(Clone)]
pub(in crate::workspace) struct ToolbarChoice {
    pub(in crate::workspace) id: ContributionId,
    pub(in crate::workspace) title: String,
    pub(in crate::workspace) icon: Icon,
    /// Whether it runs inside its plugin's own tabs rather than along every tab.
    pub(in crate::workspace) own_tabs: bool,
    /// The edge that shows it, or `None` while it is hidden.
    pub(in crate::workspace) shown: Option<Edge>,
}

/// The top edge's toolbars and the tab group of the outer dock that draws them under its tab
/// bar. Shared by the chrome, which writes it, and the outer dock's skin, which reads it.
#[derive(Default)]
pub(in crate::workspace) struct TopToolbars {
    views: RefCell<Vec<AnyView>>,
    anchor: Cell<Option<NodeId>>,
}

impl TopToolbars {
    /// Replaces the views; returns whether they changed.
    fn set_views(&self, views: Vec<AnyView>) -> bool {
        let changed = {
            let old = self.views.borrow();
            old.len() != views.len()
                || old
                    .iter()
                    .zip(&views)
                    .any(|(old, new)| old.entity_id() != new.entity_id())
        };
        if changed {
            *self.views.borrow_mut() = views;
        }
        changed
    }

    pub(in crate::workspace) fn is_empty(&self) -> bool {
        self.views.borrow().is_empty()
    }

    /// The tab group that draws the toolbars under its tab bar: the outer dock's only tab
    /// group, or `None` while the dock is split or holds no tab. Returns whether it changed.
    pub(in crate::workspace) fn set_anchor(&self, anchor: Option<NodeId>) -> bool {
        self.anchor.replace(anchor) != anchor
    }

    pub(in crate::workspace) fn anchor(&self) -> Option<NodeId> {
        self.anchor.get()
    }

    /// The row of the top edge's toolbars.
    pub(in crate::workspace) fn row(&self, cx: &App) -> Option<AnyElement> {
        let views = self.views.borrow().clone();
        (!views.is_empty()).then(|| top_row(views, cx))
    }
}

fn top_row(views: Vec<AnyView>, cx: &App) -> AnyElement {
    let theme = cx.theme();
    h_flex()
        .id("workspace-toolbars-top")
        .flex_none()
        .w_full()
        .h(px(TOOLBAR_HEIGHT))
        .min_w_0()
        .overflow_hidden()
        .gap_1()
        .px_1()
        .border_b_1()
        .border_color(theme.border)
        .bg(theme.tab_bar)
        .children(views.into_iter().map(|view| toolbar_cell(view, true)))
        .into_any_element()
}

fn edge_bar(edge: Edge, views: Vec<AnyView>, cx: &mut Context<Workspace>) -> AnyElement {
    let theme = cx.theme();
    let bar = if edge.is_horizontal() {
        h_flex()
            .w_full()
            .h(px(TOOLBAR_HEIGHT))
            .min_w_0()
            .px_1()
            .gap_1()
    } else {
        v_flex()
            .h_full()
            .w(px(TOOLBAR_WIDTH))
            .min_h_0()
            .py_1()
            .gap_1()
    };
    let bar = match edge {
        Edge::Top => bar.border_b_1(),
        Edge::Bottom => bar.border_t_1(),
        Edge::Left => bar.border_r_1(),
        Edge::Right => bar.border_l_1(),
    };
    bar.id(("workspace-toolbars", edge_key(edge)))
        .flex_none()
        .overflow_hidden()
        .border_color(theme.border)
        .bg(theme.tab_bar)
        .children(
            views
                .into_iter()
                .map(|view| toolbar_cell(view, edge.is_horizontal())),
        )
        .into_any_element()
}

/// One toolbar of an edge, drawn as a cached view so a redraw of the window reuses it. The
/// toolbars of an edge share its length, so a toolbar's spacer reaches the end of its share.
fn toolbar_cell(view: AnyView, horizontal: bool) -> AnyElement {
    let cell = div()
        .id(("workspace-toolbar", view.entity_id().as_u64() as usize))
        .flex()
        .flex_1()
        .min_w_0()
        .min_h_0()
        .overflow_hidden();
    let cell = if horizontal {
        cell.h_full()
    } else {
        cell.w_full()
    };
    cell.child(view).into_any_element()
}

fn edge_key(edge: Edge) -> usize {
    match edge {
        Edge::Top => 0,
        Edge::Bottom => 1,
        Edge::Left => 2,
        Edge::Right => 3,
    }
}

/// Places the toolbars as the arrangement says: drops the views of those it hides or turns,
/// creates the views of those it shows, and tells the workspace's observers when an edge changed,
/// so the tabs that draw their plugin's toolbar follow.
pub(in crate::workspace) fn sync(
    workspace: &mut Workspace,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let moved = workspace
        .chrome
        .toolbars_mut()
        .place(workspace.shell.current());
    for (id, factory) in workspace.chrome.toolbars().pending() {
        let view = factory(&*workspace, window, cx);
        workspace.chrome.toolbars_mut().fill(&id, view);
    }
    if workspace
        .chrome
        .toolbars()
        .publish_top(workspace.shell.current())
    {
        workspace.surfaces.outer().update(cx, |_, cx| cx.notify());
    }
    if moved {
        cx.emit(crate::WorkspaceEvent::ToolbarsChanged);
    }
}
