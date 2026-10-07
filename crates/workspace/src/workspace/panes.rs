//! Pane actions use the focused tab and its actual native placement.
use super::Workspace;
use gpui_kit::{App, Context, EntityId, Window, component::dock::InsertTarget};
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
        !self.dock.read(cx).is_locked()
            && self.item_visible(id, cx)
            && self
                .pane_for_item(id, cx)
                .is_some_and(|pane| pane.panels.len() > 1)
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
        let panes: Vec<_> = self
            .panes_in(placement, cx)
            .into_iter()
            .filter(|pane| !pane.panels.is_empty())
            .collect();
        let Some(index) = panes.iter().position(|pane| pane.node == current) else {
            return;
        };
        let target = panes[(index as isize + offset).rem_euclid(panes.len() as isize) as usize]
            .active_panel();
        if let Some(id) = target
            .and_then(|panel| self.item_for_panel(panel))
            .map(|open| open.handle.item_id())
        {
            self.activate_item_by_id(id, window, cx);
        }
    }

    pub(super) fn cycle_tab(&mut self, offset: isize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.pane_command_id(window, cx) else {
            return;
        };
        let Some(pane) = self.pane_for_item(id, cx) else {
            return;
        };
        let Some(panel) = self.panel_of(id) else {
            return;
        };
        let Some(index) = pane.panels.iter().position(|candidate| *candidate == panel) else {
            return;
        };
        let target =
            pane.panels[(index as isize + offset).rem_euclid(pane.panels.len() as isize) as usize];
        if let Some(id) = self
            .item_for_panel(target)
            .map(|open| open.handle.item_id())
        {
            self.activate_item_by_id(id, window, cx);
        }
    }

    pub fn move_active_tab(&mut self, offset: isize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.pane_command_id(window, cx) else {
            return;
        };
        let Some(pane) = self.pane_for_item(id, cx) else {
            return;
        };
        let Some(panel) = self.panel_of(id) else {
            return;
        };
        if self.dock.read(cx).is_locked() {
            return;
        }
        let target = pane
            .panels
            .iter()
            .position(|candidate| *candidate == panel)
            .and_then(|index| index.checked_add_signed(offset))
            .filter(|next| *next < pane.panels.len())
            .map(|next| InsertTarget::Tabs {
                node: pane.node,
                ix: Some(next),
                activate: true,
            });
        if let Some(target) = target {
            self.dock
                .update(cx, |dock, cx| dock.move_panel(panel, target, window, cx));
        }
    }
}
