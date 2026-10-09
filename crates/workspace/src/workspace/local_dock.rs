//! Local shell capability and native bottom visibility, sharing the tab registry.
use super::*;
use crate::LocalTerminalTarget;
impl Workspace {
    pub(crate) fn new_local_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.factory()
            .open_local(self, LocalTerminalTarget::Bottom, window, cx);
    }
    pub(crate) fn new_local_terminal_from(
        &mut self,
        id: EntityId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.item_location(id, cx).is_none() {
            return;
        }
        self.factory()
            .open_local(self, LocalTerminalTarget::Beside(id), window, cx);
    }
    pub fn set_local_terminal<T: crate::LocalTerminal>(
        &mut self,
        item: Entity<T>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ids: Vec<_> = self
            .items
            .iter()
            .filter(|open| open.local.is_some())
            .map(|open| open.handle.item_id())
            .collect();
        for id in ids {
            self.close_item_by_id(id, window, cx);
        }
        self.add_local_terminal(item, window, cx);
    }
    pub fn add_local_terminal<T: crate::LocalTerminal>(
        &mut self,
        item: Entity<T>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.add_local_terminal_at(item, LocalTerminalTarget::Bottom, window, cx);
    }
    /// Registers an owned new shell in the requested live pane.
    /// A stale anchor closes the new shell once and returns false. An already
    /// registered Item returns false without changing or closing that Item.
    pub fn add_local_terminal_at<T: crate::LocalTerminal>(
        &mut self,
        item: Entity<T>,
        target: LocalTerminalTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.is_background(item.entity_id())
            || self
                .items
                .iter()
                .any(|open| open.handle.item_id() == item.entity_id())
        {
            return false;
        }
        let destination = match target {
            LocalTerminalTarget::Bottom => self
                .selected_local(cx)
                .and_then(|open| self.item_location(open.handle.item_id(), cx))
                .filter(|(placement, _)| *placement == DockPlacement::Bottom),
            LocalTerminalTarget::Beside(anchor) => {
                let Some(destination) = self.item_location(anchor, cx) else {
                    ItemHandle::close(&item, window, cx);
                    return false;
                };
                Some(destination)
            }
        };
        let placement = destination.map_or(DockPlacement::Bottom, |(placement, _)| placement);
        if placement != DockPlacement::Center
            && self.dock.read(cx).has_dock(placement)
            && !self.dock.read(cx).is_dock_open(placement)
        {
            self.dock
                .update(cx, |dock, cx| dock.toggle_dock(placement, window, cx));
        }
        let local: Box<dyn crate::local_terminal::LocalTerminalHandle> = Box::new(item.clone());
        self.register_item(
            item,
            Some(local),
            placement,
            destination.map(|(_, node)| node),
            window,
            cx,
        );
        true
    }
    pub(super) fn local_entry(&self, id: EntityId) -> Option<&OpenItem> {
        self.items
            .iter()
            .find(|open| open.handle.item_id() == id && open.local.is_some())
    }
    pub(super) fn selected_local(&self, cx: &App) -> Option<&OpenItem> {
        let saved = self.local_entry(self.selected_local_id?)?;
        let pane = self.pane_for_item(saved.handle.item_id(), cx)?;
        self.item_for_panel(pane.active_panel()?)
            .filter(|open| open.local.is_some())
            .or(Some(saved))
    }
    pub fn toggle_local_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.set_right_panel_maximized(false, cx);
        if self.dock.read(cx).has_dock(DockPlacement::Bottom) {
            let was_open = self.dock.read(cx).is_dock_open(DockPlacement::Bottom);
            let focused_bottom = self
                .command_item(window, cx)
                .and_then(|item| self.item_location(item.item_id(), cx))
                .is_some_and(|(placement, _)| placement == DockPlacement::Bottom);
            self.dock.update(cx, |dock, cx| {
                dock.toggle_dock(DockPlacement::Bottom, window, cx)
            });
            if was_open && focused_bottom {
                self.focus_central(window, cx);
            } else if !was_open {
                let selected = self
                    .selected_local(cx)
                    .filter(|open| {
                        self.item_location(open.handle.item_id(), cx)
                            .is_some_and(|(placement, _)| placement == DockPlacement::Bottom)
                    })
                    .map(|open| open.handle.item_id())
                    .or_else(|| {
                        self.items
                            .iter()
                            .find(|open| {
                                self.item_location(open.handle.item_id(), cx).is_some_and(
                                    |(placement, _)| placement == DockPlacement::Bottom,
                                )
                            })
                            .map(|open| open.handle.item_id())
                    });
                if let Some(id) = selected {
                    self.activate_item_by_id(id, window, cx);
                }
            }
        } else {
            self.new_local_terminal(window, cx);
        }
        cx.emit(WorkspaceEvent::ItemsChanged);
        cx.notify();
    }
    pub(super) fn reconcile_bottom_visibility(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.dock.read(cx).is_dock_open(DockPlacement::Bottom)
            && self.items.iter().any(|open| {
                open.dock_item.read(cx).contains_focus(window, cx)
                    && self
                        .item_location(open.handle.item_id(), cx)
                        .is_some_and(|(placement, _)| placement == DockPlacement::Bottom)
            })
        {
            self.focus_central(window, cx);
        }
    }
    pub fn close_local_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.selected_local(cx).map(|open| open.handle.item_id()) {
            self.close_item_by_id(id, window, cx);
        }
    }
    pub fn change_local_directory(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        if self.selected_local(cx).is_none() {
            self.new_local_terminal(window, cx);
        }
        if self
            .selected_local(cx)
            .and_then(|open| self.item_location(open.handle.item_id(), cx))
            .is_some_and(|(placement, _)| placement == DockPlacement::Bottom)
            && !self.dock.read(cx).is_dock_open(DockPlacement::Bottom)
        {
            self.toggle_local_terminal(window, cx);
        }
        self.selected_local(cx)
            .and_then(|open| open.local.as_ref())
            .ok_or("Local terminal is unavailable")?
            .change_directory(path, window, cx)
    }
}
#[cfg(test)]
mod tests;
