//! Local shell capability and native bottom visibility, sharing the tab registry.
use super::*;
pub(super) type LocalOpener = Rc<dyn Fn(&mut Workspace, &mut Window, &mut Context<Workspace>)>;
impl Workspace {
    pub fn set_local_terminal_opener(
        &mut self,
        opener: impl Fn(&mut Workspace, &mut Window, &mut Context<Workspace>) + 'static,
    ) {
        self.local_opener = Some(Rc::new(opener));
    }
    pub(crate) fn new_local_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(opener) = self.local_opener.clone() {
            opener(self, window, cx);
        }
    }
    pub(crate) fn new_local_terminal_from(
        &mut self,
        id: EntityId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(target) = self.item_location(id, cx) else {
            return;
        };
        let previous = self.local_open_target.replace(target);
        self.new_local_terminal(window, cx);
        self.local_open_target = previous;
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
        let target = self.local_open_target.or_else(|| {
            self.selected_local(cx)
                .and_then(|open| self.item_location(open.handle.item_id(), cx))
                .filter(|(placement, _)| *placement == DockPlacement::Bottom)
        });
        let placement = target.map_or(DockPlacement::Bottom, |(placement, _)| placement);
        if placement == DockPlacement::Bottom
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
            target.map(|(_, node)| node),
            window,
            cx,
        );
    }
    pub(super) fn local_entry(&self, id: EntityId) -> Option<&OpenItem> {
        self.items
            .iter()
            .find(|open| open.handle.item_id() == id && open.local.is_some())
    }
    pub(super) fn selected_local(&self, cx: &App) -> Option<&OpenItem> {
        let saved = self.local_entry(self.selected_local_id?)?;
        let (placement, node) = self.item_location(saved.handle.item_id(), cx)?;
        let dock = self.dock.read(cx);
        let PaneRef::Tabs { panels, active_ix } = dock.layout(placement)?.find_node(node)?.kind()
        else {
            return Some(saved);
        };
        let active = panels.get(active_ix)?;
        self.items
            .iter()
            .find(|open| {
                PanelId::from(open.dock_item.entity_id()) == *active && open.local.is_some()
            })
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
    pub(super) fn reconcile_empty_regions(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let empty: Vec<_> = [
            DockPlacement::Bottom,
            DockPlacement::Left,
            DockPlacement::Right,
        ]
        .into_iter()
        .filter(|placement| {
            self.dock.read(cx).has_dock(*placement)
                && !super::items::region_has_panels(self.dock.read(cx), *placement)
        })
        .collect();
        for placement in empty {
            if placement == DockPlacement::Bottom {
                self.local_terminal_height = self.dock.read(cx).dock_size(placement);
            }
            self.dock
                .update(cx, |dock, cx| dock.remove_dock(placement, window, cx));
        }
    }
    pub(super) fn reconcile_bottom_visibility(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.dock.read(cx).is_dock_open(DockPlacement::Bottom)
            && self.items.iter().any(|open| {
                self.item_location(open.handle.item_id(), cx)
                    .is_some_and(|(placement, _)| placement == DockPlacement::Bottom)
                    && open.dock_item.read(cx).contains_focus(window, cx)
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
