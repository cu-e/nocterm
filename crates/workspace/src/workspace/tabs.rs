//! Close scopes resolve current native placement and visual pane order.
use super::*;
impl Workspace {
    pub(crate) fn tabs_to_close(
        &self,
        id: EntityId,
        scope: TabCloseScope,
        cx: &App,
    ) -> Vec<EntityId> {
        let Some((placement, node)) = self.item_location(id, cx) else {
            return vec![];
        };
        let dock = self.dock.read(cx);
        let Some(tree) = dock.layout(placement) else {
            return vec![];
        };
        let panels = if scope == TabCloseScope::All {
            tree.node_ids()
                .into_iter()
                .filter_map(|node| tree.find_node(node))
                .flat_map(|node| match node.kind() {
                    PaneRef::Tabs { panels, .. } => panels.to_vec(),
                    _ => vec![],
                })
                .collect::<Vec<_>>()
        } else {
            let Some(node) = tree.find_node(node) else {
                return vec![];
            };
            let PaneRef::Tabs { panels, .. } = node.kind() else {
                return vec![];
            };
            let Some(clicked) = panels
                .iter()
                .position(|panel| Some(*panel) == self.panel_of(id))
            else {
                return vec![];
            };
            panels
                .iter()
                .enumerate()
                .filter(|(ix, _)| match scope {
                    TabCloseScope::Current => *ix == clicked,
                    TabCloseScope::Others => *ix != clicked,
                    TabCloseScope::Left => *ix < clicked,
                    TabCloseScope::Right => *ix > clicked,
                    TabCloseScope::All => true,
                })
                .map(|(_, panel)| *panel)
                .collect()
        };
        panels
            .into_iter()
            .filter_map(|panel| {
                self.items
                    .iter()
                    .find(|open| PanelId::from(open.dock_item.entity_id()) == panel)
                    .map(|open| open.handle.item_id())
            })
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
