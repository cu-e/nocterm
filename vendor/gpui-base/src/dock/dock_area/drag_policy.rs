// Modified by Nocterm contributors; see NOCTERM.md. Licensed under Apache-2.0.
//! Opt-in drag permission protects the last visible panel across open regions.
use super::*;
impl DockArea {
    /// Allow a region's last tab to move when another open region remains visible.
    /// Defaults to false; this permission never relaxes closing or explicit locks.
    pub fn set_last_panel_drag_between_regions(
        &mut self,
        allowed: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.last_panel_drag_between_regions == allowed {
            return;
        }
        self.last_panel_drag_between_regions = allowed;
        self.reconcile(window, cx);
    }
    pub(super) fn can_drag_last_panel_from(&self, node: NodeId, cx: &App) -> bool {
        if !self.last_panel_drag_between_regions {
            return false;
        }
        let Some(source) = self.placement_of_node(node) else {
            return false;
        };
        [
            DockPlacement::Center,
            DockPlacement::Bottom,
            DockPlacement::Left,
            DockPlacement::Right,
        ]
        .into_iter()
        .any(|placement| {
            placement != source
                && (placement == DockPlacement::Center || self.is_dock_open(placement))
                && self.layout(placement).is_some_and(|tree| {
                    tree.node_ids().into_iter().any(|node| {
                        match tree.find_node(node).map(PaneNode::kind) {
                            Some(PaneRef::Tabs { panels, .. }) => panels.iter().any(|panel| {
                                self.panels
                                    .get(panel)
                                    .is_some_and(|panel| panel.visible(cx))
                            }),
                            _ => false,
                        }
                    })
                })
        })
    }
}
