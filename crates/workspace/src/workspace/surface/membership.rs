//! Actual foreground and off-dock membership; no parent entity or context access.
use super::*;
use gpui_kit::component::dock::NodeId;
use tabs::{TabEntry, TabTransition};

pub(crate) struct SurfaceContent {
    tabs: tabs::Tabs,
    background: background::BackgroundSessions,
}
impl SurfaceContent {
    pub(in crate::workspace) fn new(tabs: tabs::Tabs) -> Self {
        Self {
            tabs,
            background: Default::default(),
        }
    }
    pub(super) fn frame(&self, view: AnyView, cx: &App) -> SurfaceFrame {
        SurfaceFrame {
            view,
            visible: self.tabs.has_visible_tabs(cx),
            local_visible: self.tabs.local_visible(cx),
        }
    }
    pub(in crate::workspace) fn tabs(&self) -> &tabs::Tabs {
        &self.tabs
    }
    pub(in crate::workspace) fn contains(&self, id: EntityId) -> bool {
        self.tabs.contains(id) || self.background.contains(id)
    }
    pub(in crate::workspace) fn item_handle(&self, id: EntityId) -> Option<Rc<dyn ItemHandle>> {
        self.tabs
            .item_handle(id)
            .or_else(|| self.background.item_handle(id))
    }
    pub(in crate::workspace) fn background_contains(&self, id: EntityId) -> bool {
        self.background.contains(id)
    }
    pub(in crate::workspace) fn terminals(&self, cx: &App) -> Vec<crate::TerminalEntry> {
        let mut entries = self.tabs.foreground_terminals(cx);
        entries.extend(self.background.terminals(cx));
        let active = self
            .tabs
            .last_command()
            .filter(|id| entries.iter().any(|entry| entry.item == *id))
            .or_else(|| self.tabs.active_item().map(ItemHandle::item_id));
        for entry in &mut entries {
            entry.active = active == Some(entry.item);
        }
        entries
    }
    pub(in crate::workspace) fn register(
        &mut self,
        entry: TabEntry,
        placement: DockPlacement,
        target: Option<NodeId>,
        focus: bool,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<EntityId> {
        let previous = self.tabs.selection_at(placement, target, cx);
        let id = self.tabs.register(entry, placement, target, window, cx)?;
        if !focus {
            if let Some(previous) = previous {
                self.tabs.retain_selection(previous, window, cx);
            }
            self.tabs.admit_without_focus(id);
        }
        Some(id)
    }
    pub(super) fn admit(
        &mut self,
        entry: TabEntry,
        destination: &admission::SurfaceItemAdmission,
        focus: bool,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<Option<EntityId>, ContextSelectionError> {
        let (placement, node) = destination.destination(&self.tabs, cx)?;
        self.tabs.ensure_region_open(placement, window, cx);
        Ok(self.register(entry, placement, node, focus, window, cx))
    }
    pub(in crate::workspace) fn insert_background(&mut self, entry: TabEntry) -> EntityId {
        let id = entry.id();
        self.background.insert(entry);
        id
    }
    pub(in crate::workspace) fn promote(
        &mut self,
        id: EntityId,
        focus: bool,
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        let Some(entry) = self.background.take(id) else {
            return false;
        };
        let target = self
            .tabs
            .active_location(cx)
            .filter(|(placement, _)| *placement == DockPlacement::Center)
            .map(|(_, node)| node);
        self.register(entry, DockPlacement::Center, target, focus, window, cx)
            .is_some()
    }
    pub(in crate::workspace) fn close_background(
        &mut self,
        id: EntityId,
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        let Some(entry) = self.background.take(id) else {
            return false;
        };
        entry.close(window, cx);
        true
    }
    pub(in crate::workspace) fn close(
        &mut self,
        id: EntityId,
        focused: bool,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<TabTransition> {
        self.tabs.close(id, focused, window, cx)
    }
    pub(in crate::workspace) fn close_all(&mut self, window: &mut Window, cx: &mut App) {
        let ids: Vec<_> = self
            .tabs
            .snapshots(cx)
            .into_iter()
            .map(|tab| tab.id())
            .collect();
        for id in ids {
            self.tabs.close(id, false, window, cx);
        }
        for id in self.background.ids() {
            self.close_background(id, window, cx);
        }
    }
    pub(in crate::workspace) fn background_changed(
        &mut self,
        id: EntityId,
        session: Option<SessionContext>,
        cx: &mut App,
    ) -> bool {
        self.background.changed(id, session, cx)
    }
    pub(super) fn activate(
        &mut self,
        id: EntityId,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<TabTransition> {
        if !self.tabs.item_visible(id, cx) {
            return None;
        }
        self.tabs.activate(id, window, cx)
    }
    pub(super) fn changed(
        &mut self,
        id: EntityId,
        session: Option<SessionContext>,
        cx: &mut App,
    ) -> Option<TabTransition> {
        self.tabs.changed(id, session, cx)
    }
    pub(super) fn focused(&mut self, id: EntityId) -> Option<TabTransition> {
        self.tabs.focused(id)
    }
    pub(super) fn native_changed(
        &mut self,
        window: &mut Window,
        cx: &mut App,
    ) -> Vec<WorkspaceEvent> {
        self.tabs.native_changed(window, cx)
    }
    pub(super) fn toggle_bottom(
        &mut self,
        focused: Option<EntityId>,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<tabs::TabFocus> {
        self.tabs.toggle_bottom(focused, window, cx)
    }
    pub(super) fn split(
        &mut self,
        id: EntityId,
        placement: gpui_kit::component::Placement,
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        self.tabs.split(id, placement, window, cx)
    }
    pub(in crate::workspace) fn active_item(&self) -> Option<&dyn ItemHandle> {
        self.tabs().active_item()
    }
    pub(in crate::workspace) fn tab_location(
        &self,
        id: EntityId,
        cx: &App,
    ) -> Option<(DockPlacement, NodeId)> {
        self.tabs().item_location(id, cx)
    }
    pub(crate) fn tabs_to_close(
        &self,
        id: EntityId,
        scope: TabCloseScope,
        cx: &App,
    ) -> Vec<EntityId> {
        self.tabs().tabs_to_close(id, scope, cx)
    }
    pub(crate) fn can_split_item(&self, id: EntityId, cx: &App) -> bool {
        self.tabs().can_split(id, cx)
    }
    pub(crate) fn tab_groups_to_join(
        &self,
        id: EntityId,
        cx: &App,
    ) -> Vec<(EntityId, SharedString)> {
        self.tabs().tab_groups_to_join(id, cx)
    }
    pub(crate) fn is_tab_grouped(&self, id: EntityId) -> bool {
        self.tabs().is_tab_grouped(id)
    }
    pub(super) fn rename(
        &self,
        id: EntityId,
        selection: ContextSelectionReceipt,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.tabs.rename(id, selection, window, cx);
    }
    pub(in crate::workspace) fn move_item(
        &mut self,
        id: EntityId,
        offset: isize,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.tabs.move_tab(id, offset, window, cx);
    }
    pub(in crate::workspace) fn change_directory(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<(), crate::LocalDirectoryError> {
        self.tabs()
            .selected_local_handle(cx)
            .ok_or(crate::LocalDirectoryError::Unavailable)?
            .change_directory(path, window, cx)
    }
    pub(crate) fn new_tab_group(&mut self, id: EntityId, window: &mut Window, cx: &mut App) {
        self.tabs.new_tab_group(id, window, cx);
    }
    pub(crate) fn group_tab_with(
        &mut self,
        id: EntityId,
        other: EntityId,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.tabs.group_tab_with(id, other, window, cx);
    }
    pub(crate) fn ungroup_tab(&mut self, id: EntityId, window: &mut Window, cx: &mut App) {
        self.tabs.ungroup_tab(id, window, cx);
    }
    pub(crate) fn dissolve_tab_group(&mut self, id: EntityId, window: &mut Window, cx: &mut App) {
        self.tabs.dissolve_tab_group(id, window, cx);
    }
    pub(in crate::workspace) fn group_program(
        &mut self,
        parent: Option<EntityId>,
        opened_from: usize,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.tabs.group_program(parent, opened_from, window, cx);
    }
}

pub(in crate::workspace) struct SurfaceFrame {
    view: AnyView,
    visible: bool,
    local_visible: bool,
}
impl SurfaceFrame {
    pub(in crate::workspace) fn view(&self) -> AnyView {
        self.view.clone()
    }
    pub(in crate::workspace) fn visible(&self) -> bool {
        self.visible
    }
    pub(in crate::workspace) fn local_visible(&self) -> bool {
        self.local_visible
    }
}
