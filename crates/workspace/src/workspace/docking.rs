//! Native pane queries and region lifecycle; placement and order stay in DockArea.
use super::*;
use gpui_kit::component::dock::NodeId;

pub(super) const TAB_PLACEMENTS: [DockPlacement; 4] = [
    DockPlacement::Center,
    DockPlacement::Bottom,
    DockPlacement::Left,
    DockPlacement::Right,
];

/// An ephemeral snapshot of one native pane, never retained as workspace state.
pub(super) struct DockPane {
    pub(super) placement: DockPlacement,
    pub(super) node: NodeId,
    pub(super) panels: Vec<PanelId>,
    active_ix: usize,
}
impl DockPane {
    pub(super) fn active_panel(&self) -> Option<PanelId> {
        self.panels.get(self.active_ix).copied()
    }
}
impl Workspace {
    pub(super) fn panel_of(&self, id: EntityId) -> Option<PanelId> {
        self.items
            .iter()
            .find(|open| open.handle.item_id() == id)
            .map(|open| PanelId::from(open.dock_item.entity_id()))
    }
    pub(super) fn item_for_panel(&self, panel: PanelId) -> Option<&OpenItem> {
        self.items
            .iter()
            .find(|open| PanelId::from(open.dock_item.entity_id()) == panel)
    }
    pub(super) fn item_location(&self, id: EntityId, cx: &App) -> Option<(DockPlacement, NodeId)> {
        let panel = self.panel_of(id)?;
        let dock = self.dock.read(cx);
        TAB_PLACEMENTS.into_iter().find_map(|placement| {
            Some((placement, dock.layout(placement)?.find_panel_node(panel)?))
        })
    }
    pub(super) fn item_visible(&self, id: EntityId, cx: &App) -> bool {
        self.item_location(id, cx).is_some_and(|(placement, _)| {
            placement == DockPlacement::Center || self.dock.read(cx).is_dock_open(placement)
        })
    }
    pub(super) fn pane_at(
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
        Some(DockPane {
            placement,
            node,
            panels: panels.to_vec(),
            active_ix,
        })
    }
    pub(super) fn pane_for_item(&self, id: EntityId, cx: &App) -> Option<DockPane> {
        let (placement, node) = self.item_location(id, cx)?;
        self.pane_at(placement, node, cx)
    }
    pub(super) fn panes_in(&self, placement: DockPlacement, cx: &App) -> Vec<DockPane> {
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
                Some(DockPane {
                    placement,
                    node,
                    panels: panels.to_vec(),
                    active_ix,
                })
            })
            .collect()
    }
    pub(super) fn reconcile_empty_regions(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for placement in TAB_PLACEMENTS
            .into_iter()
            .filter(|placement| *placement != DockPlacement::Center)
        {
            // Actual registered panels count even when their region is closed.
            if self.dock.read(cx).has_dock(placement)
                && !region_has_panels(self.dock.read(cx), placement)
            {
                if placement == DockPlacement::Bottom {
                    self.local_terminal_height = self.dock.read(cx).dock_size(placement);
                }
                self.dock
                    .update(cx, |dock, cx| dock.remove_dock(placement, window, cx));
            }
        }
    }
}

fn region_has_panels(dock: &DockArea, placement: DockPlacement) -> bool {
    dock.layout(placement).is_some_and(|tree| {
        tree.node_ids().into_iter().any(|node| {
            matches!(
                tree.find_node(node).map(|node| node.kind()),
                Some(PaneRef::Tabs { panels, .. }) if !panels.is_empty()
            )
        })
    })
}
