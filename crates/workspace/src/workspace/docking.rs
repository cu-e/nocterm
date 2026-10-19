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
    pub(super) fn new(
        placement: DockPlacement,
        node: NodeId,
        panels: Vec<PanelId>,
        active_ix: usize,
    ) -> Self {
        Self {
            placement,
            node,
            panels,
            active_ix,
        }
    }

    pub(super) fn active_panel(&self) -> Option<PanelId> {
        self.panels.get(self.active_ix).copied()
    }
}
