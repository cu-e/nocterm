//! Bottom local terminals: lifetime, attachment, selection and directory routing.
use super::*;

pub(super) type LocalOpener = Rc<dyn Fn(&mut Workspace, &mut Window, &mut Context<Workspace>)>;
pub(super) struct BottomEntry {
    pub item: Entity<crate::dock_item::DockItem>,
    pub handle: Rc<dyn ItemHandle>,
    pub local: Box<dyn crate::local_terminal::LocalTerminalHandle>,
    _subscription: Subscription,
    _focus_subscription: Subscription,
}
pub(super) struct BottomTerminal {
    pub entries: Vec<BottomEntry>,
    pub selected: EntityId,
    pub attached: bool,
    pub height: Pixels,
}
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
    /// Replace the bottom session, retaining the original single-terminal API.
    pub fn set_local_terminal<T: crate::LocalTerminal>(
        &mut self,
        item: Entity<T>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_all_local_terminals(window, cx);
        self.add_local_terminal(item, window, cx);
    }
    /// Append a session to the existing bottom tab group and display it.
    pub fn add_local_terminal<T: crate::LocalTerminal>(
        &mut self,
        item: Entity<T>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .local_terminal
            .as_ref()
            .is_some_and(|local| !local.attached)
        {
            self.show_local_terminal(window, cx);
        }
        let mut cwd = item.read(cx).cwd(cx);
        let id = item.entity_id();
        let subscription =
            cx.subscribe_in(
                &item,
                window,
                move |this, item, event, window, cx| match event {
                    ItemEvent::Changed => {
                        if let Some(entry) = this.local_entry(id) {
                            entry.item.update(cx, |_, cx| cx.notify());
                        }
                        let now = item.read(cx).cwd(cx);
                        if now != cwd {
                            cwd = now;
                            if this
                                .selected_local(cx)
                                .is_some_and(|entry| entry.handle.item_id() == id)
                            {
                                cx.emit(WorkspaceEvent::LocalDirectoryChanged);
                            }
                        }
                        cx.emit(WorkspaceEvent::ItemsChanged);
                        cx.notify();
                    }
                    ItemEvent::CloseRequested => this.close_local_terminal_by_id(id, window, cx),
                },
            );
        let handle: Rc<dyn ItemHandle> = Rc::new(item.clone());
        let workspace = cx.weak_entity();
        let header_origin = self.local_header_click_origin.clone();
        let dock_item = cx.new(|item_cx| {
            crate::dock_item::DockItem::new(
                handle.clone(),
                workspace,
                true,
                header_origin,
                window,
                item_cx,
            )
        });
        let focus = dock_item.read(cx).container_focus_handle();
        let focus_subscription = cx.on_focus_in(&focus, window, move |this, _, cx| {
            if let Some(local) = this.local_terminal.as_mut() {
                local.selected = id;
            }
            if this.last_command_item != Some(id) {
                this.last_command_item = Some(id);
                cx.emit(WorkspaceEvent::ItemsChanged);
            }
            cx.emit(WorkspaceEvent::LocalDirectoryChanged);
        });
        let height = self.local_terminal.as_ref().map_or_else(
            || rems(cx.design().layout.local_terminal_height).to_pixels(window.rem_size()),
            |local| local.height,
        );
        self.local_terminal
            .get_or_insert_with(|| BottomTerminal {
                entries: vec![],
                selected: id,
                attached: true,
                height,
            })
            .entries
            .push(BottomEntry {
                item: dock_item.clone(),
                handle: handle.clone(),
                local: Box::new(item),
                _subscription: subscription,
                _focus_subscription: focus_subscription,
            });
        let panel = PanelId::from(dock_item.entity_id());
        // Base appends to the first group. Move into the selected bottom group
        // afterwards so adding while a split group is zoomed stays in that group.
        let target = self.selected_local(cx).and_then(|entry| {
            self.dock
                .read(cx)
                .layout(DockPlacement::Bottom)?
                .find_panel_node(PanelId::from(entry.item.entity_id()))
        });
        self.dock.update(cx, |dock, cx| {
            dock.add_panel_view(
                panel_handle(dock_item),
                DockPlacement::Bottom,
                Some(height),
                window,
                cx,
            );
            if let Some(node) = target {
                dock.move_panel(
                    panel,
                    InsertTarget::Tabs {
                        node,
                        ix: None,
                        activate: true,
                    },
                    window,
                    cx,
                );
            }
        });
        self.local_terminal.as_mut().unwrap().selected = id;
        self.last_command_item = Some(id);
        window.focus(&handle.focus_handle(cx), cx);
        cx.emit(WorkspaceEvent::LocalDirectoryChanged);
        cx.emit(WorkspaceEvent::ItemsChanged);
        cx.notify();
    }
    pub(super) fn local_entry(&self, id: EntityId) -> Option<&BottomEntry> {
        self.local_terminal
            .as_ref()?
            .entries
            .iter()
            .find(|entry| entry.handle.item_id() == id)
    }
    pub(super) fn local_placement(&self, entry: &BottomEntry, cx: &App) -> Option<DockPlacement> {
        let panel = PanelId::from(entry.item.entity_id());
        [
            DockPlacement::Bottom,
            DockPlacement::Center,
            DockPlacement::Left,
            DockPlacement::Right,
        ]
        .into_iter()
        .find(|placement| {
            self.dock
                .read(cx)
                .layout(*placement)
                .is_some_and(|tree| tree.find_panel_node(panel).is_some())
        })
    }
    pub(super) fn local_entry_visible(&self, id: EntityId, cx: &App) -> bool {
        self.local_entry(id)
            .and_then(|entry| self.local_placement(entry, cx))
            .is_some_and(|placement| {
                placement == DockPlacement::Center || self.dock.read(cx).is_dock_open(placement)
            })
    }
    /// Selection comes from the actual group containing the most recently focused
    /// local tab. The saved id also retains selection while all views are detached.
    pub(super) fn selected_local(&self, cx: &App) -> Option<&BottomEntry> {
        let local = self.local_terminal.as_ref()?;
        let saved = self.local_entry(local.selected)?;
        if !local.attached {
            return Some(saved);
        }
        let placement = self.local_placement(saved, cx)?;
        let tree = self.dock.read(cx).layout(placement)?;
        let node = tree.find_panel_node(PanelId::from(saved.item.entity_id()))?;
        let PaneRef::Tabs {
            panels, active_ix, ..
        } = tree.find_node(node)?.kind()
        else {
            return None;
        };
        let active = panels.get(active_ix)?;
        local
            .entries
            .iter()
            .find(|entry| PanelId::from(entry.item.entity_id()) == *active)
            .or(Some(saved))
    }
    pub fn toggle_local_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.set_right_panel_maximized(false, cx);
        if self
            .local_terminal
            .as_ref()
            .is_some_and(|local| local.attached)
        {
            self.hide_local_terminal(window, cx);
        } else if self.local_terminal.is_some() {
            self.show_local_terminal(window, cx);
        } else {
            self.new_local_terminal(window, cx);
        }
        cx.notify();
    }
    pub(super) fn schedule_hidden_dock_detach(&mut self, window: &Window, cx: &mut Context<Self>) {
        if self.dock_hide_queued
            || !self
                .local_terminal
                .as_ref()
                .is_some_and(|local| local.attached)
            || !self.dock.read(cx).has_dock(DockPlacement::Bottom)
            || self.dock.read(cx).is_dock_open(DockPlacement::Bottom)
        {
            return;
        }
        self.dock_hide_queued = true;
        let workspace = cx.weak_entity();
        cx.defer_in(window, move |_, window, cx| {
            let _ = workspace.update(cx, |workspace, cx| {
                workspace.dock_hide_queued = false;
                if workspace
                    .local_terminal
                    .as_ref()
                    .is_some_and(|local| local.attached)
                    && workspace.dock.read(cx).has_dock(DockPlacement::Bottom)
                    && !workspace.dock.read(cx).is_dock_open(DockPlacement::Bottom)
                {
                    workspace.hide_local_terminal(window, cx);
                }
            });
        });
    }
    fn hide_local_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let selected = self.selected_local(cx).map(|entry| entry.handle.item_id());
        let Some(local) = self.local_terminal.as_mut().filter(|local| local.attached) else {
            return;
        };
        if let Some(selected) = selected {
            local.selected = selected;
        }
        let mut order = Vec::new();
        for placement in [
            DockPlacement::Bottom,
            DockPlacement::Center,
            DockPlacement::Left,
            DockPlacement::Right,
        ] {
            if let Some(tree) = self.dock.read(cx).layout(placement) {
                collect_panels(tree.root(), &mut order);
            }
        }
        local.entries.sort_by_key(|entry| {
            order
                .iter()
                .position(|panel| *panel == PanelId::from(entry.item.entity_id()))
                .unwrap_or(usize::MAX)
        });
        local.attached = false;
        local.height = self
            .dock
            .read(cx)
            .dock_size(DockPlacement::Bottom)
            .unwrap_or(local.height);
        for entry in &local.entries {
            entry
                .item
                .update(cx, |item, _| item.detach_without_closing());
        }
        let items: Vec<_> = local
            .entries
            .iter()
            .map(|entry| entry.item.clone())
            .collect();
        self.dock.update(cx, |dock, cx| {
            for item in items {
                dock.remove_panel(item, window, cx);
            }
            if dock.is_empty(DockPlacement::Bottom, cx) {
                dock.remove_dock(DockPlacement::Bottom, window, cx);
            }
        });
        self.focus_central(window, cx);
        cx.emit(WorkspaceEvent::ItemsChanged);
        cx.notify();
    }
    fn show_local_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(local) = self.local_terminal.as_mut().filter(|local| !local.attached) else {
            return;
        };
        local.attached = true;
        let height = local.height;
        let items: Vec<_> = local
            .entries
            .iter()
            .map(|entry| entry.item.clone())
            .collect();
        let Some(selected) = local
            .entries
            .iter()
            .find(|entry| entry.handle.item_id() == local.selected)
        else {
            return;
        };
        let panel = PanelId::from(selected.item.entity_id());
        let focus = selected.handle.focus_handle(cx);
        self.last_command_item = Some(selected.handle.item_id());
        self.dock.update(cx, |dock, cx| {
            for item in items {
                dock.add_panel_view(
                    panel_handle(item),
                    DockPlacement::Bottom,
                    Some(height),
                    window,
                    cx,
                );
            }
            dock.select_panel(panel, window, cx);
        });
        window.focus(&focus, cx);
        cx.emit(WorkspaceEvent::ItemsChanged);
        cx.notify();
    }
    /// Close the selected local tab; closing the last tab removes the bottom dock.
    pub fn close_local_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.selected_local(cx).map(|entry| entry.handle.item_id()) {
            self.close_local_terminal_by_id(id, window, cx);
        }
    }
    pub(crate) fn close_local_terminal_by_id(
        &mut self,
        id: EntityId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let active = self.selected_local(cx).map(|entry| entry.handle.item_id());
        let Some(local) = self.local_terminal.as_mut() else {
            return;
        };
        let Some(index) = local
            .entries
            .iter()
            .position(|entry| entry.handle.item_id() == id)
        else {
            return;
        };
        let entry = local.entries.remove(index);
        let last = local.entries.is_empty();
        if local.selected == id && !last {
            local.selected = local.entries[index.min(local.entries.len() - 1)]
                .handle
                .item_id();
        }
        let attached = local.attached;
        if last {
            self.local_terminal = None;
        }
        self.dock.update(cx, |dock, cx| {
            if attached {
                dock.remove_panel(entry.item.clone(), window, cx);
            }
            if dock.is_empty(DockPlacement::Bottom, cx) {
                dock.remove_dock(DockPlacement::Bottom, window, cx);
            }
        });
        entry.handle.close(window, cx);
        if self.last_command_item == Some(id) {
            self.last_command_item = None;
        }
        if last {
            self.focus_central(window, cx);
        } else if active == Some(id)
            && attached
            && let Some(selected) = self.selected_local(cx)
        {
            let id = selected.handle.item_id();
            let focus = selected.handle.focus_handle(cx);
            self.last_command_item = Some(id);
            window.focus(&focus, cx);
        }
        cx.emit(WorkspaceEvent::LocalDirectoryChanged);
        cx.emit(WorkspaceEvent::ItemsChanged);
        cx.notify();
    }
    fn close_all_local_terminals(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ids: Vec<_> = self
            .local_terminal
            .as_ref()
            .into_iter()
            .flat_map(|local| local.entries.iter().map(|entry| entry.handle.item_id()))
            .collect();
        for id in ids {
            self.close_local_terminal_by_id(id, window, cx);
        }
    }
    fn focus_central(&self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(
            &self
                .active_item()
                .map_or_else(|| self.focus_handle.clone(), |item| item.focus_handle(cx)),
            cx,
        );
    }
    pub fn change_local_directory(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        if self.local_terminal.is_none() {
            self.toggle_local_terminal(window, cx);
        }
        if self
            .local_terminal
            .as_ref()
            .is_some_and(|local| !local.attached)
        {
            self.show_local_terminal(window, cx);
        }
        self.selected_local(cx)
            .ok_or("Local terminal is unavailable")?
            .local
            .change_directory(path, window, cx)
    }
}

fn collect_panels(node: &gpui_kit::component::dock::PaneNode, order: &mut Vec<PanelId>) {
    match node.kind() {
        PaneRef::Tabs { panels, .. } => order.extend_from_slice(panels),
        PaneRef::Split { children, .. } => {
            for child in children {
                collect_panels(child, order);
            }
        }
    }
}

#[cfg(test)]
mod tests;
