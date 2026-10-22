//! Admission, cached metadata and registered-first close ownership.
use super::*;
impl Tabs {
    pub(in crate::workspace) fn register(
        &mut self,
        entry: TabEntry,
        placement: DockPlacement,
        target: Option<NodeId>,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<EntityId> {
        let id = entry.id();
        if self.contains(id) {
            return None;
        }
        let dock_item = entry.dock_item.clone();
        self.entries.push(entry);
        let height = (placement == DockPlacement::Bottom).then(|| self.local.height(window, cx));
        self.topology
            .add(dock_item, placement, target, height, window, cx);
        Some(id)
    }
    pub(in crate::workspace) fn changed(
        &mut self,
        id: EntityId,
        session: Option<SessionContext>,
        cx: &mut App,
    ) -> Option<TabTransition> {
        let entry = self.entries.iter_mut().find(|entry| entry.id() == id)?;
        entry.session = session;
        entry.dock_item.update(cx, |_, cx| cx.notify());
        let mut cwd_changed = false;
        if let Some(local) = &entry.local {
            let now = local.cwd(cx);
            cwd_changed = now != entry.cwd;
            entry.cwd = now;
        }
        let mut events = Vec::new();
        if cwd_changed && self.selected_local_id(cx) == Some(id) {
            events.push(WorkspaceEvent::LocalDirectoryChanged);
        }
        events.extend(self.announce_session());
        events.push(WorkspaceEvent::ItemsChanged);
        Some(TabTransition::new(
            events,
            TabFocus::Preserve,
            Vec::new(),
            true,
        ))
    }
    pub(in crate::workspace) fn close(
        &mut self,
        id: EntityId,
        focused: bool,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<TabTransition> {
        let index = self.entries.iter().position(|entry| entry.id() == id)?;
        let previous = self.active_central_id;
        let location = self.item_location(id, cx);
        let closed = self.entries.remove(index).detach();
        self.groups
            .leave(PanelId::from(closed.dock_item.entity_id()));
        self.topology.remove(closed.dock_item, window, cx);
        self.reconcile_empty_regions(window, cx);
        closed.handle.close(window, cx);
        let replacement = location
            .and_then(|(placement, node)| self.topology.pane(placement, node, cx))
            .and_then(|pane| pane.active_panel())
            .and_then(|panel| self.item_for_panel(panel))
            .map(TabEntry::id);
        if self.local.selected() == Some(id) {
            let selected = replacement
                .filter(|id| self.local_entry(*id).is_some())
                .or_else(|| {
                    self.entries
                        .iter()
                        .find(|entry| entry.local.is_some())
                        .map(TabEntry::id)
                });
            self.local.select(selected);
        }
        let mut before = Vec::new();
        if previous == Some(id) {
            self.active_central_id = replacement
                .filter(|id| self.entry(*id).is_some_and(|entry| entry.local.is_none()))
                .or_else(|| {
                    self.entries
                        .iter()
                        .find(|entry| entry.local.is_none())
                        .map(TabEntry::id)
                });
            before.push(WorkspaceEvent::ActiveItemChanged);
            before.extend(self.announce_session());
        }
        if self.last_command_id == Some(id) {
            self.last_command_id = None;
        }
        let focus = if focused {
            replacement
                .filter(|id| self.item_visible(*id, cx))
                .map_or(TabFocus::Central, TabFocus::Activate)
        } else {
            TabFocus::Preserve
        };
        Some(TabTransition::new(
            before,
            focus,
            vec![
                WorkspaceEvent::LocalDirectoryChanged,
                WorkspaceEvent::ItemsChanged,
            ],
            true,
        ))
    }
    pub(in crate::workspace) fn admit_without_focus(&mut self, id: EntityId) {
        if self.local_entry(id).is_some() {
            if self.local.selected().is_none() {
                self.local.select(Some(id));
            }
        } else if self.active_central_id.is_none() {
            self.mark_active(id);
        }
    }
    pub(in crate::workspace) fn mark_active(&mut self, id: EntityId) -> Vec<WorkspaceEvent> {
        if !self.entry(id).is_some_and(|entry| entry.local.is_none())
            || self.active_central_id == Some(id)
        {
            return Vec::new();
        }
        self.active_central_id = Some(id);
        let mut events = vec![WorkspaceEvent::ActiveItemChanged];
        events.extend(self.announce_session());
        events
    }
    fn announce_session(&mut self) -> Option<WorkspaceEvent> {
        let session = self.active_entry().and_then(|entry| entry.session.clone());
        let unchanged = match (&session, &self.announced_session) {
            (Some(now), Some(before)) => now.same_as(before),
            (None, None) => true,
            _ => false,
        };
        if unchanged {
            return None;
        }
        self.announced_session = session;
        Some(WorkspaceEvent::ActiveSessionChanged)
    }
    pub(in crate::workspace) fn native_changed(
        &mut self,
        window: &mut Window,
        cx: &mut App,
    ) -> Vec<WorkspaceEvent> {
        self.reconcile_empty_regions(window, cx);
        self.regroup(window, cx);
        let focused = self
            .entries
            .iter()
            .find(|entry| entry.dock_item.read(cx).contains_focus(window, cx))
            .map(TabEntry::id);
        focused.map_or_else(Vec::new, |id| self.mark_active(id))
    }
    fn reconcile_empty_regions(&mut self, window: &mut Window, cx: &mut App) {
        for placement in TAB_PLACEMENTS
            .into_iter()
            .filter(|placement| *placement != DockPlacement::Center)
        {
            if self.topology.region_present(placement, cx)
                && self
                    .topology
                    .panes(placement, cx)
                    .iter()
                    .all(|pane| pane.panels.is_empty())
            {
                if placement == DockPlacement::Bottom {
                    self.local
                        .remember_height(self.topology.region_size(placement, cx));
                }
                self.topology.remove_region(placement, window, cx);
            }
        }
    }
    pub(in crate::workspace) fn hidden_bottom_has_focus(&self, window: &Window, cx: &App) -> bool {
        !self.region_open(DockPlacement::Bottom, cx)
            && self.entries.iter().any(|entry| {
                entry.dock_item.read(cx).contains_focus(window, cx)
                    && self
                        .item_location(entry.id(), cx)
                        .is_some_and(|(placement, _)| placement == DockPlacement::Bottom)
            })
    }
    pub(in crate::workspace) fn rename(
        &self,
        id: EntityId,
        selection: ContextSelectionReceipt,
        window: &mut Window,
        cx: &mut App,
    ) {
        if let Some(entry) = self.entry(id) {
            entry
                .dock_item
                .update(cx, |item, cx| item.start_alias(selection, window, cx));
        }
    }
}
