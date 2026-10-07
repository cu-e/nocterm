//! Close scopes resolve current native placement and visual pane order.
use super::*;
impl Workspace {
    pub(crate) fn tabs_to_close(
        &self,
        id: EntityId,
        scope: TabCloseScope,
        cx: &App,
    ) -> Vec<EntityId> {
        let Some(pane) = self.pane_for_item(id, cx) else {
            return vec![];
        };
        let panels = if scope == TabCloseScope::All {
            self.panes_in(pane.placement, cx)
                .into_iter()
                .flat_map(|pane| pane.panels)
                .collect::<Vec<_>>()
        } else {
            let Some(clicked_panel) = self.panel_of(id) else {
                return vec![];
            };
            let Some(clicked) = pane.panels.iter().position(|panel| *panel == clicked_panel) else {
                return vec![];
            };
            pane.panels
                .into_iter()
                .enumerate()
                .filter(|(ix, _)| match scope {
                    TabCloseScope::Current => *ix == clicked,
                    TabCloseScope::Others => *ix != clicked,
                    TabCloseScope::Left => *ix < clicked,
                    TabCloseScope::Right => *ix > clicked,
                    TabCloseScope::All => true,
                })
                .map(|(_, panel)| panel)
                .collect()
        };
        panels
            .into_iter()
            .filter_map(|panel| self.item_for_panel(panel).map(|open| open.handle.item_id()))
            .collect()
    }
    pub fn close_tabs(
        &mut self,
        id: EntityId,
        scope: TabCloseScope,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for id in self.tabs_to_close(id, scope, cx) {
            self.close_item_by_id(id, window, cx);
        }
    }
    pub(super) fn close_active_tabs(
        &mut self,
        scope: TabCloseScope,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(id) = self.command_item(window, cx).map(|item| item.item_id()) {
            self.close_tabs(id, scope, window, cx);
        }
    }
}
