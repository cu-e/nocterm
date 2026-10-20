//! Immutable shell observations; no native or owner mutation handle escapes.
use super::*;
use gpui_kit::component::dock::{DockAreaState, NodeId};
impl Workspace {
    pub fn items(&self) -> impl Iterator<Item = &dyn ItemHandle> {
        self.tabs.central_items()
    }
    pub fn active_item(&self) -> Option<&dyn ItemHandle> {
        self.tabs.active_item()
    }
    pub fn find_item<T: Item>(&self) -> Option<Entity<T>> {
        self.tabs.find()
    }
    pub fn connection_directory(&self) -> Option<Rc<dyn crate::ConnectionDirectory>> {
        self.ports.directory()
    }
    pub fn active_session(&self, _: &App) -> Option<SessionContext> {
        self.tabs.active_session()
    }
    pub fn connected_session_for_host(&self, host: &HostAddress) -> Option<SessionContext> {
        self.tabs.connected_session(host)
    }
    pub fn tabs(&self, cx: &App) -> Vec<TabSnapshot> {
        self.tabs.snapshots(cx)
    }
    pub fn tab_panes(&self, placement: DockPlacement, cx: &App) -> Vec<TabPane> {
        self.tabs.panes(placement, cx)
    }
    pub fn dock_region_present(&self, placement: DockPlacement, cx: &App) -> bool {
        self.tabs.region_present(placement, cx)
    }
    pub fn dock_region_open(&self, placement: DockPlacement, cx: &App) -> bool {
        self.tabs.region_open(placement, cx)
    }
    pub fn dock_region_size(&self, placement: DockPlacement, cx: &App) -> Option<Pixels> {
        self.tabs.region_size(placement, cx)
    }
    pub fn zoomed_pane(&self, cx: &App) -> Option<NodeId> {
        self.tabs.zoomed(cx)
    }
    pub fn dock_layout(&self, cx: &App) -> DockAreaState {
        self.tabs.dock_layout(cx)
    }
    pub fn selected_local_terminal(&self, cx: &App) -> Option<EntityId> {
        self.tabs.selected_local_id(cx)
    }
    pub fn local_terminal_cwd(&self, cx: &App) -> Option<PathBuf> {
        self.tabs.local_cwd(cx)
    }
    pub fn local_terminal_is_visible(&self, cx: &App) -> bool {
        self.tabs.local_visible(cx)
    }
    pub fn is_background(&self, id: EntityId) -> bool {
        self.background.contains(id)
    }
    pub fn right_panel_is_available(&self) -> bool {
        self.right_panel.available()
    }
    pub fn right_panel_is_open(&self) -> bool {
        self.right_panel.open()
    }
    pub fn right_panel_is_maximized(&self) -> bool {
        self.right_panel.maximized()
    }
    pub fn new_tab_menu_is_open(&self) -> bool {
        self.chrome.new_tab_open()
    }
    pub fn sidebar_is_open(&self) -> bool {
        self.sidebar.visible()
    }
    pub fn sides_swapped(&self, cx: &App) -> bool {
        self.body.swapped(cx)
    }
    pub fn tab_group_items(&self, id: EntityId) -> Vec<EntityId> {
        self.tabs.tab_group_items(id)
    }
    pub fn tab_group_count(&self) -> usize {
        self.tabs.tab_group_count()
    }
    pub(crate) fn tab_groups_to_join(
        &self,
        id: EntityId,
        cx: &App,
    ) -> Vec<(EntityId, SharedString)> {
        self.tabs.tab_groups_to_join(id, cx)
    }
    pub(crate) fn is_tab_grouped(&self, id: EntityId) -> bool {
        self.tabs.is_tab_grouped(id)
    }
    /// Previews the current visual close scope without exposing the dock's mutable state.
    pub fn tabs_to_close(&self, id: EntityId, scope: TabCloseScope, cx: &App) -> Vec<EntityId> {
        self.tabs.tabs_to_close(id, scope, cx)
    }
    pub(crate) fn can_split_item(&self, id: EntityId, cx: &App) -> bool {
        self.tabs.can_split(id, cx)
    }
    pub fn can_split_active(&self, window: &Window, cx: &App) -> bool {
        self.command_item(window, cx)
            .is_some_and(|item| self.tabs.can_split(item.item_id(), cx))
    }
    pub fn tab_location(&self, id: EntityId, cx: &App) -> Option<(DockPlacement, NodeId)> {
        self.tabs.item_location(id, cx)
    }
    pub fn tab_is_visible(&self, id: EntityId, cx: &App) -> bool {
        self.tabs.item_visible(id, cx)
    }
    pub(super) fn factory(&self) -> openers::Factory {
        self.ports.factory()
    }
    /// Focused visible Item wins; a captured menu target never falls back when stale.
    pub fn command_item(&self, window: &Window, cx: &App) -> Option<Rc<dyn ItemHandle>> {
        if let Some(item) = self.tabs.focused_item(window, cx) {
            return Some(item);
        }
        let menu = self
            .chrome
            .command_target(window, self.tabs.last_command(), cx);
        self.tabs.command_item(menu, cx)
    }
    pub fn command_available(&self, action: &dyn Action, window: &Window, cx: &App) -> bool {
        !self.focus_handle.is_focused(window)
            && self
                .command_item(window, cx)
                .is_some_and(|item| window.is_action_available_in(action, &item.focus_handle(cx)))
    }
    pub fn command_title(&self, window: &Window, cx: &App) -> Option<SharedString> {
        self.tabs
            .title(self.command_item(window, cx)?.item_id(), cx)
    }
    pub fn active_terminal(&self, cx: &App) -> Option<EntityId> {
        let terminals = self.terminals(cx);
        self.tabs
            .last_command()
            .filter(|id| terminals.iter().any(|entry| entry.item == *id))
            .or_else(|| {
                self.active_item()
                    .filter(|item| item.terminal_capabilities(cx).is_some())
                    .map(|item| item.item_id())
            })
    }
    pub fn terminals(&self, cx: &App) -> Vec<crate::TerminalEntry> {
        let mut entries = self.tabs.foreground_terminals(cx);
        entries.extend(self.background.terminals(cx));
        let active = self
            .tabs
            .last_command()
            .filter(|id| entries.iter().any(|entry| entry.item == *id))
            .or_else(|| self.active_item().map(|item| item.item_id()));
        for entry in &mut entries {
            entry.active = active == Some(entry.item);
        }
        entries
    }
}
