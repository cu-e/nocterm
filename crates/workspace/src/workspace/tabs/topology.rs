//! The single native placement tree and its two subscription lifetimes.
use super::super::docking::{DockPane, TAB_PLACEMENTS};
use super::super::*;
use gpui_kit::component::dock::{DockAreaState, InsertTarget, NodeId};

pub(super) struct DockTopology {
    dock: Entity<DockArea>,
    _events: Subscription,
    _observer: Subscription,
}
impl DockTopology {
    pub(super) fn new(window: &mut Window, cx: &mut Context<Workspace>) -> Self {
        let (dock, skin) = DockSkin::dock_area("workspace", None, window, cx);
        skin.set_panel_style(PanelStyle::TabBar, cx);
        skin.set_close_button_visible(false, cx);
        skin.set_toggle_button_visible(false, cx);
        dock.update(cx, |dock, cx| {
            dock.set_closed_bottom_strip_visible(false, cx);
            dock.set_last_panel_drag_between_regions(true, window, cx);
        });
        let events = cx.subscribe_in(&dock, window, |this, _, _: &DockEvent, window, cx| {
            this.native_dock_changed(window, cx)
        });
        let observer = cx.observe_in(&dock, window, |this, _, window, cx| {
            this.native_dock_visibility_changed(window, cx)
        });
        Self {
            dock,
            _events: events,
            _observer: observer,
        }
    }
    pub(super) fn view(&self) -> AnyView {
        self.dock.clone().into()
    }
    pub(super) fn locked(&self, cx: &App) -> bool {
        self.dock.read(cx).is_locked()
    }
    pub(super) fn region_present(&self, placement: DockPlacement, cx: &App) -> bool {
        self.dock.read(cx).has_dock(placement)
    }
    pub(super) fn region_open(&self, placement: DockPlacement, cx: &App) -> bool {
        self.dock.read(cx).is_dock_open(placement)
    }
    pub(super) fn region_size(&self, placement: DockPlacement, cx: &App) -> Option<Pixels> {
        self.dock.read(cx).dock_size(placement)
    }
    pub(super) fn zoomed(&self, cx: &App) -> Option<NodeId> {
        self.dock.read(cx).zoomed_group()
    }
    pub(super) fn dump(&self, cx: &App) -> DockAreaState {
        self.dock.read(cx).dump(cx)
    }
    pub(super) fn location(&self, panel: PanelId, cx: &App) -> Option<(DockPlacement, NodeId)> {
        let dock = self.dock.read(cx);
        TAB_PLACEMENTS.into_iter().find_map(|placement| {
            Some((placement, dock.layout(placement)?.find_panel_node(panel)?))
        })
    }
    pub(super) fn pane(
        &self,
        placement: DockPlacement,
        node: NodeId,
        cx: &App,
    ) -> Option<DockPane> {
        let dock = self.dock.read(cx);
        let PaneRef::Tabs { panels, active_ix } = dock.layout(placement)?.find_node(node)?.kind()
        else {
            return None;
        };
        Some(DockPane::new(placement, node, panels.to_vec(), active_ix))
    }
    pub(super) fn panes(&self, placement: DockPlacement, cx: &App) -> Vec<DockPane> {
        let dock = self.dock.read(cx);
        let Some(tree) = dock.layout(placement) else {
            return vec![];
        };
        tree.node_ids()
            .into_iter()
            .filter_map(|node| {
                let PaneRef::Tabs { panels, active_ix } = tree.find_node(node)?.kind() else {
                    return None;
                };
                Some(DockPane::new(placement, node, panels.to_vec(), active_ix))
            })
            .collect()
    }
    pub(super) fn add(
        &self,
        item: Entity<crate::dock_item::DockItem>,
        placement: DockPlacement,
        target: Option<NodeId>,
        height: Option<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.dock.update(cx, |dock, cx| {
            let panel = panel_handle(item);
            if let Some(node) = target {
                dock.add_panel_view_to_group(panel, node, window, cx);
            } else {
                dock.add_panel_view(panel, placement, height, window, cx);
            }
        });
    }
    pub(super) fn remove(
        &self,
        item: Entity<crate::dock_item::DockItem>,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.dock
            .update(cx, |dock, cx| dock.remove_panel(item, window, cx));
    }
    pub(super) fn select(&self, panel: PanelId, window: &mut Window, cx: &mut App) {
        self.dock
            .update(cx, |dock, cx| dock.select_panel(panel, window, cx));
    }
    pub(super) fn move_panel(
        &self,
        panel: PanelId,
        target: InsertTarget,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.dock
            .update(cx, |dock, cx| dock.move_panel(panel, target, window, cx));
    }
    pub(super) fn toggle(&self, placement: DockPlacement, window: &mut Window, cx: &mut App) {
        self.dock
            .update(cx, |dock, cx| dock.toggle_dock(placement, window, cx));
    }
    pub(super) fn remove_region(
        &self,
        placement: DockPlacement,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.dock
            .update(cx, |dock, cx| dock.remove_dock(placement, window, cx));
    }
}
