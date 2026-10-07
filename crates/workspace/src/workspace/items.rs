//! One lifecycle for every user tab; the native dock owns placement and order.
use super::*;
use gpui_kit::component::dock::NodeId;

pub(super) const TAB_PLACEMENTS: [DockPlacement; 4] = [
    DockPlacement::Center,
    DockPlacement::Bottom,
    DockPlacement::Left,
    DockPlacement::Right,
];
impl Workspace {
    pub fn add_item<T: Item>(
        &mut self,
        item: Entity<T>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = self
            .active_item
            .and_then(|ix| self.items.get(ix))
            .and_then(|open| self.item_location(open.handle.item_id(), cx))
            .filter(|(placement, _)| *placement == DockPlacement::Center);
        self.register_item(
            item,
            None,
            DockPlacement::Center,
            target.map(|(_, node)| node),
            window,
            cx,
        );
    }
    pub(super) fn register_item<T: Item>(
        &mut self,
        item: Entity<T>,
        local: Option<Box<dyn crate::local_terminal::LocalTerminalHandle>>,
        placement: DockPlacement,
        target: Option<NodeId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = item.entity_id();
        let mut cwd = local.as_ref().and_then(|local| local.cwd(cx));
        let subscription =
            cx.subscribe_in(
                &item,
                window,
                move |this, item, event, window, cx| match event {
                    ItemEvent::Changed => {
                        if let Some(open) = this
                            .items
                            .iter_mut()
                            .find(|open| open.handle.item_id() == id)
                        {
                            open.session = item.read(cx).session(cx);
                            open.dock_item.update(cx, |_, cx| cx.notify());
                            if let Some(local) = &open.local {
                                let now = local.cwd(cx);
                                if now != cwd {
                                    cwd = now;
                                    if this
                                        .selected_local(cx)
                                        .is_some_and(|open| open.handle.item_id() == id)
                                    {
                                        cx.emit(WorkspaceEvent::LocalDirectoryChanged);
                                    }
                                }
                            }
                        }
                        this.announce_session(cx);
                        cx.emit(WorkspaceEvent::ItemsChanged);
                        cx.notify();
                    }
                    ItemEvent::CloseRequested => this.close_item_by_id(id, window, cx),
                },
            );
        let is_local = local.is_some();
        let session = item.read(cx).session(cx);
        let handle: Rc<dyn ItemHandle> = Rc::new(item);
        let workspace = cx.weak_entity();
        let origin = self.local_header_click_origin.clone();
        let dock_item = cx.new(|cx| {
            crate::dock_item::DockItem::new(handle.clone(), workspace, is_local, origin, window, cx)
        });
        let focus = dock_item.read(cx).container_focus_handle();
        let focus_subscription = cx.on_focus_in(&focus, window, move |this, _, cx| {
            if this.last_command_item != Some(id) {
                this.last_command_item = Some(id);
                cx.emit(WorkspaceEvent::ItemsChanged);
            }
            if this.local_entry(id).is_some() {
                this.selected_local_id = Some(id);
                cx.emit(WorkspaceEvent::LocalDirectoryChanged);
            }
            this.mark_active(id, cx);
        });
        self.items.push(OpenItem {
            handle,
            session,
            dock_item: dock_item.clone(),
            local,
            _subscription: subscription,
            _focus_subscription: focus_subscription,
        });
        let height = (placement == DockPlacement::Bottom).then(|| {
            self.local_terminal_height.unwrap_or_else(|| {
                rems(cx.design().layout.local_terminal_height).to_pixels(window.rem_size())
            })
        });
        self.dock.update(cx, |dock, cx| {
            let panel = panel_handle(dock_item);
            if let Some(node) = target {
                dock.add_panel_view_to_group(panel, node, window, cx);
            } else {
                dock.add_panel_view(panel, placement, height, window, cx);
            }
        });
        self.activate_item_by_id(id, window, cx);
        cx.emit(WorkspaceEvent::ItemsChanged);
    }
    /// Central-context Items in registration order; indices are central ordinals.
    pub fn items(&self) -> impl Iterator<Item = &dyn ItemHandle> {
        self.items
            .iter()
            .filter(|open| open.local.is_none())
            .map(|open| open.handle.as_ref())
    }
    pub fn set_connection_directory(&mut self, directory: Rc<dyn crate::ConnectionDirectory>) {
        self.connection_directory = Some(directory);
    }
    pub fn connection_directory(&self) -> Option<Rc<dyn crate::ConnectionDirectory>> {
        self.connection_directory.clone()
    }
    pub fn active_item(&self) -> Option<&dyn ItemHandle> {
        self.active_item
            .and_then(|ix| self.items.get(ix))
            .map(|open| open.handle.as_ref())
    }
    pub fn find_item<T: Item>(&self) -> Option<Entity<T>> {
        self.items
            .iter()
            .find_map(|open| open.handle.view().downcast::<T>().ok())
    }
    pub fn activate_item(&mut self, ordinal: usize, window: &mut Window, cx: &mut Context<Self>) {
        let id = self.items().nth(ordinal).map(|item| item.item_id());
        if let Some(id) = id {
            self.activate_item_by_id(id, window, cx);
        }
    }
    pub fn activate_item_by_id(
        &mut self,
        id: EntityId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(index) = self
            .items
            .iter()
            .position(|open| open.handle.item_id() == id)
        else {
            return false;
        };
        if !self.item_visible(id, cx) {
            return false;
        }
        self.set_right_panel_maximized(false, cx);
        let open = &self.items[index];
        let panel = PanelId::from(open.dock_item.entity_id());
        let focus = open.handle.focus_handle(cx);
        self.dock
            .update(cx, |dock, cx| dock.select_panel(panel, window, cx));
        self.last_command_item = Some(id);
        self.mark_active(id, cx);
        if self.items[index].local.is_some() {
            self.selected_local_id = Some(id);
            cx.emit(WorkspaceEvent::LocalDirectoryChanged);
        }
        window.focus(&focus, cx);
        cx.notify();
        true
    }
    pub fn close_item(&mut self, ordinal: usize, window: &mut Window, cx: &mut Context<Self>) {
        let id = self.items().nth(ordinal).map(|item| item.item_id());
        if let Some(id) = id {
            self.close_item_by_id(id, window, cx);
        }
    }
    pub(crate) fn close_item_by_id(
        &mut self,
        id: EntityId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self
            .items
            .iter()
            .position(|open| open.handle.item_id() == id)
        else {
            return;
        };
        let previous = self.active_item().map(|item| item.item_id());
        let focused = self
            .command_item(window, cx)
            .is_some_and(|item| item.item_id() == id);
        let location = self.item_location(id, cx);
        if let Some(height) = self.dock.read(cx).dock_size(DockPlacement::Bottom) {
            self.local_terminal_height = Some(height);
        }
        let closed = self.items.remove(index);
        self.tab_groups
            .leave(PanelId::from(closed.dock_item.entity_id()));
        self.dock.update(cx, |dock, cx| {
            dock.remove_panel(closed.dock_item.clone(), window, cx);
            for placement in [
                DockPlacement::Bottom,
                DockPlacement::Left,
                DockPlacement::Right,
            ] {
                if dock.has_dock(placement) && !region_has_panels(dock, placement) {
                    dock.remove_dock(placement, window, cx);
                }
            }
        });
        closed.handle.close(window, cx);
        let replacement = location.and_then(|(placement, node)| {
            let dock = self.dock.read(cx);
            let PaneRef::Tabs { panels, active_ix } =
                dock.layout(placement)?.find_node(node)?.kind()
            else {
                return None;
            };
            let panel = panels.get(active_ix)?;
            self.items
                .iter()
                .find(|open| PanelId::from(open.dock_item.entity_id()) == *panel)
                .map(|open| open.handle.item_id())
        });
        if self.selected_local_id == Some(id) {
            self.selected_local_id = replacement
                .filter(|id| self.local_entry(*id).is_some())
                .or_else(|| {
                    self.items
                        .iter()
                        .find(|open| open.local.is_some())
                        .map(|open| open.handle.item_id())
                });
        }
        if previous == Some(id) {
            self.active_item = replacement
                .and_then(|id| {
                    self.items
                        .iter()
                        .position(|open| open.handle.item_id() == id && open.local.is_none())
                })
                .or_else(|| self.items.iter().position(|open| open.local.is_none()));
            cx.emit(WorkspaceEvent::ActiveItemChanged);
            self.announce_session(cx);
        } else {
            self.active_item = previous.and_then(|id| {
                self.items
                    .iter()
                    .position(|open| open.handle.item_id() == id)
            });
        }
        if self.last_command_item == Some(id) {
            self.last_command_item = None;
        }
        if focused {
            if let Some(replacement) = replacement.filter(|id| self.item_visible(*id, cx)) {
                self.activate_item_by_id(replacement, window, cx);
            } else {
                self.focus_central(window, cx);
            }
        }
        cx.emit(WorkspaceEvent::LocalDirectoryChanged);
        cx.emit(WorkspaceEvent::ItemsChanged);
        cx.notify();
    }
    pub(crate) fn mark_active(&mut self, id: EntityId, cx: &mut Context<Self>) {
        let ix = self
            .items
            .iter()
            .position(|open| open.handle.item_id() == id && open.local.is_none());
        if ix.is_some() && ix != self.active_item {
            self.active_item = ix;
            cx.emit(WorkspaceEvent::ActiveItemChanged);
            self.announce_session(cx);
            cx.notify();
        }
    }
    pub(crate) fn item_removed(
        &mut self,
        id: EntityId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_item_by_id(id, window, cx);
    }
    pub(super) fn item_location(&self, id: EntityId, cx: &App) -> Option<(DockPlacement, NodeId)> {
        let panel = self.panel_of(id)?;
        let dock = self.dock.read(cx);
        TAB_PLACEMENTS.into_iter().find_map(|placement| {
            Some((placement, dock.layout(placement)?.find_panel_node(panel)?))
        })
    }
    pub(super) fn item_visible(&self, id: EntityId, cx: &App) -> bool {
        self.item_location(id, cx).is_some_and(|(placement, _)| {
            placement == DockPlacement::Center || self.dock.read(cx).is_dock_open(placement)
        })
    }
    pub fn rename_active_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.command_item(window, cx).map(|item| item.item_id())
            && let Some(open) = self.items.iter().find(|open| open.handle.item_id() == id)
        {
            open.dock_item
                .update(cx, |item, cx| item.start_alias(window, cx));
        }
    }
    pub(super) fn focus_central(&self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(
            &self
                .active_item()
                .filter(|item| self.item_visible(item.item_id(), cx))
                .map_or_else(|| self.focus_handle.clone(), |item| item.focus_handle(cx)),
            cx,
        );
    }
}

pub(super) fn region_has_panels(dock: &DockArea, placement: DockPlacement) -> bool {
    dock.layout(placement).is_some_and(|tree| tree.node_ids().into_iter().any(|node| matches!(tree.find_node(node).map(|node| node.kind()), Some(PaneRef::Tabs { panels, .. }) if !panels.is_empty())))
}
