//! Arranging tabs in panes: splitting, moving between and within them.

use gpui_kit::{
    App, Context, Window,
    component::dock::{DockPlacement, InsertTarget, PaneRef, PanelId},
};

use super::Workspace;

impl Workspace {
    /// Move the active tab beside its current pane without recreating its Item.
    pub fn split_active(
        &mut self,
        placement: gpui_kit::component::Placement,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = self.active_item().map(|item| item.item_id()) else {
            return;
        };
        self.split_item(id, placement, window, cx);
    }

    /// Splitting moves a tab out of a pane which retains another tab.
    pub fn can_split_active(&self, cx: &App) -> bool {
        let Some(open) = self.active_item.and_then(|ix| self.items.get(ix)) else {
            return false;
        };
        let panel = PanelId::from(open.dock_item.entity_id());
        self.dock
            .read(cx)
            .layout(DockPlacement::Center)
            .and_then(|tree| {
                tree.find_panel_node(panel)
                    .and_then(|node| tree.find_node(node))
            })
            .is_some_and(
                |node| matches!(node.kind(), PaneRef::Tabs { panels, .. } if panels.len() > 1),
            )
    }

    pub fn split_item(
        &mut self,
        id: gpui_kit::EntityId,
        placement: gpui_kit::component::Placement,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(open) = self.items.iter().find(|item| item.handle.item_id() == id) else {
            return;
        };
        let panel = PanelId::from(open.dock_item.entity_id());
        let node = self
            .dock
            .read(cx)
            .layout(DockPlacement::Center)
            .and_then(|tree| tree.find_panel_node(panel));
        if let Some(node) = node {
            // A tab split off to sit beside its group no longer moves with it.
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
        }
        cx.notify();
    }

    pub fn focus_pane(&mut self, offset: isize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(open) = self.active_item.and_then(|ix| self.items.get(ix)) else {
            return;
        };
        let panel = PanelId::from(open.dock_item.entity_id());
        let target = self
            .dock
            .read(cx)
            .layout(DockPlacement::Center)
            .and_then(|tree| {
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
                let current = tree.find_panel_node(panel)?;
                let ix = nodes.iter().position(|(node, _)| *node == current)?;
                Some(nodes[(ix as isize + offset).rem_euclid(nodes.len() as isize) as usize].1)
            });
        if let Some(target) = target
            && let Some(ix) = self
                .items
                .iter()
                .position(|open| PanelId::from(open.dock_item.entity_id()) == target)
        {
            self.activate_item(ix, window, cx);
        }
    }

    pub(super) fn cycle_tab(&mut self, offset: isize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(open) = self.active_item.and_then(|ix| self.items.get(ix)) else {
            return;
        };
        let panel = PanelId::from(open.dock_item.entity_id());
        let target = self
            .dock
            .read(cx)
            .layout(DockPlacement::Center)
            .and_then(|tree| {
                let node = tree.find_panel_node(panel)?;
                let PaneRef::Tabs { panels, .. } = tree.find_node(node)?.kind() else {
                    return None;
                };
                let ix = panels.iter().position(|id| *id == panel)?;
                Some(panels[(ix as isize + offset).rem_euclid(panels.len() as isize) as usize])
            });
        if let Some(target) = target
            && let Some(ix) = self
                .items
                .iter()
                .position(|open| PanelId::from(open.dock_item.entity_id()) == target)
        {
            self.activate_item(ix, window, cx);
        }
    }

    /// Reorder the active tab in its pane. Pointer drag uses this same dock tree.
    pub fn move_active_tab(&mut self, offset: isize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(open) = self.active_item.and_then(|ix| self.items.get(ix)) else {
            return;
        };
        let panel = PanelId::from(open.dock_item.entity_id());
        let target = self
            .dock
            .read(cx)
            .layout(DockPlacement::Center)
            .and_then(|tree| {
                let node = tree.find_panel_node(panel)?;
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
