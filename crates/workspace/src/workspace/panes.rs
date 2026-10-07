//! Pane actions use the focused tab and its actual native placement.
use super::Workspace;
use gpui_kit::{
    App, Context, EntityId, Window,
    component::dock::{InsertTarget, PaneRef},
};
impl Workspace {
    fn pane_command_id(&self, window: &Window, cx: &App) -> Option<EntityId> {
        self.command_item(window, cx).map(|item| item.item_id())
    }
    pub fn split_active(
        &mut self,
        placement: gpui_kit::component::Placement,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(id) = self.pane_command_id(window, cx) {
            self.split_item(id, placement, window, cx);
        }
    }
    pub fn can_split_active(&self, window: &Window, cx: &App) -> bool {
        self.command_item(window, cx)
            .is_some_and(|item| self.can_split_item(item.item_id(), cx))
    }
    pub(crate) fn can_split_item(&self, id: EntityId, cx: &App) -> bool {
        let Some((placement, node)) = self.item_location(id, cx) else {
            return false;
        };
        !self.dock.read(cx).is_locked()
            && self.item_visible(id, cx)
            && self
                .dock
                .read(cx)
                .layout(placement)
                .and_then(|tree| tree.find_node(node))
                .is_some_and(
                    |node| matches!(node.kind(), PaneRef::Tabs { panels, .. } if panels.len() > 1),
                )
    }
    pub fn split_item(
        &mut self,
        id: EntityId,
        placement: gpui_kit::component::Placement,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.can_split_item(id, cx) {
            return;
        }
        let Some((_, node)) = self.item_location(id, cx) else {
            return;
        };
        let Some(panel) = self.panel_of(id) else {
            return;
        };
        self.tab_groups.leave(panel);
        self.dock.update(cx, |dock, cx| {
            dock.move_panel(
                panel,
                InsertTarget::Split {
                    node,
                    placement,
                    size: None,
                },
                window,
                cx,
            )
        });
        cx.notify();
    }
    pub fn focus_pane(&mut self, offset: isize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.pane_command_id(window, cx) else {
            return;
        };
        let Some((placement, current)) = self.item_location(id, cx) else {
            return;
        };
        let target = self.dock.read(cx).layout(placement).and_then(|tree| {
            let nodes: Vec<_> = tree
                .node_ids()
                .into_iter()
                .filter_map(|node| match tree.find_node(node)?.kind() {
                    PaneRef::Tabs { panels, active_ix } if !panels.is_empty() => {
                        Some((node, panels[active_ix.min(panels.len() - 1)]))
                    }
                    _ => None,
                })
                .collect();
            let index = nodes.iter().position(|(node, _)| *node == current)?;
            Some(nodes[(index as isize + offset).rem_euclid(nodes.len() as isize) as usize].1)
        });
        if let Some(id) = target.and_then(|panel| {
            self.items
                .iter()
                .find(|open| Some(panel) == self.panel_of(open.handle.item_id()))
                .map(|open| open.handle.item_id())
        }) {
            self.activate_item_by_id(id, window, cx);
        }
    }
    pub(super) fn cycle_tab(&mut self, offset: isize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.pane_command_id(window, cx) else {
            return;
        };
        let Some((placement, node)) = self.item_location(id, cx) else {
            return;
        };
        let target = self.dock.read(cx).layout(placement).and_then(|tree| {
            let PaneRef::Tabs { panels, .. } = tree.find_node(node)?.kind() else {
                return None;
            };
            let ix = panels
                .iter()
                .position(|panel| Some(*panel) == self.panel_of(id))?;
            Some(panels[(ix as isize + offset).rem_euclid(panels.len() as isize) as usize])
        });
        if let Some(id) = target.and_then(|panel| {
            self.items
                .iter()
                .find(|open| Some(panel) == self.panel_of(open.handle.item_id()))
                .map(|open| open.handle.item_id())
        }) {
            self.activate_item_by_id(id, window, cx);
        }
    }
    pub fn move_active_tab(&mut self, offset: isize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.pane_command_id(window, cx) else {
            return;
        };
        let Some((placement, node)) = self.item_location(id, cx) else {
            return;
        };
        let Some(panel) = self.panel_of(id) else {
            return;
        };
        if self.dock.read(cx).is_locked() {
            return;
        }
        let target = self.dock.read(cx).layout(placement).and_then(|tree| {
            let PaneRef::Tabs { panels, .. } = tree.find_node(node)?.kind() else {
                return None;
            };
            let ix = panels.iter().position(|id| *id == panel)?;
            let next = ix.checked_add_signed(offset)?;
            (next < panels.len()).then_some(InsertTarget::Tabs {
                node,
                ix: Some(next),
                activate: true,
            })
        });
        if let Some(target) = target {
            self.dock
                .update(cx, |dock, cx| dock.move_panel(panel, target, window, cx));
        }
    }
}
