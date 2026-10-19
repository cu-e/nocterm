//! Visible selection, local-region focus and concrete native pane commands.
use super::*;
use gpui_kit::component::{Placement, dock::InsertTarget};

impl Tabs {
    pub(in crate::workspace) fn activate(
        &mut self,
        id: EntityId,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<TabTransition> {
        let entry = self.entry(id)?;
        if !self.item_visible(id, cx) {
            return None;
        }
        let panel = entry.panel();
        let focus = entry.handle.focus_handle(cx);
        let local = entry.local.is_some();
        self.topology.select(panel, window, cx);
        self.last_command_id = Some(id);
        let mut events = self.mark_active(id);
        if local {
            self.local.select(Some(id));
            events.push(WorkspaceEvent::LocalDirectoryChanged);
        }
        Some(TabTransition::new(
            events,
            TabFocus::Direct(focus),
            Vec::new(),
            true,
        ))
    }
    pub(in crate::workspace) fn focused(&mut self, id: EntityId) -> Option<TabTransition> {
        if !self.contains(id) {
            return None;
        }
        let mut events = Vec::new();
        if self.last_command_id != Some(id) {
            self.last_command_id = Some(id);
            events.push(WorkspaceEvent::ItemsChanged);
        }
        if self.local_entry(id).is_some() {
            self.local.select(Some(id));
            events.push(WorkspaceEvent::LocalDirectoryChanged);
        }
        events.extend(self.mark_active(id));
        Some(TabTransition::new(
            events,
            TabFocus::Preserve,
            Vec::new(),
            true,
        ))
    }
    pub(in crate::workspace) fn ensure_region_open(
        &self,
        placement: DockPlacement,
        window: &mut Window,
        cx: &mut App,
    ) {
        if placement != DockPlacement::Center
            && self.region_present(placement, cx)
            && !self.region_open(placement, cx)
        {
            self.topology.toggle(placement, window, cx);
        }
    }
    pub(in crate::workspace) fn local_visible(&self, cx: &App) -> bool {
        self.region_open(DockPlacement::Bottom, cx)
            && self.entries.iter().any(|entry| {
                self.item_location(entry.id(), cx)
                    .is_some_and(|(placement, _)| placement == DockPlacement::Bottom)
            })
    }
    pub(in crate::workspace) fn toggle_bottom(
        &self,
        focused: Option<EntityId>,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<TabFocus> {
        if !self.region_present(DockPlacement::Bottom, cx) {
            return None;
        }
        let was_open = self.region_open(DockPlacement::Bottom, cx);
        let focused_bottom = focused
            .and_then(|id| self.item_location(id, cx))
            .is_some_and(|(placement, _)| placement == DockPlacement::Bottom);
        self.topology.toggle(DockPlacement::Bottom, window, cx);
        if was_open && focused_bottom {
            return Some(TabFocus::Central);
        }
        if was_open {
            return Some(TabFocus::Preserve);
        }
        let selected = self
            .selected_local_id(cx)
            .filter(|id| {
                self.item_location(*id, cx)
                    .is_some_and(|(placement, _)| placement == DockPlacement::Bottom)
            })
            .or_else(|| {
                self.entries
                    .iter()
                    .find(|entry| {
                        self.item_location(entry.id(), cx)
                            .is_some_and(|(placement, _)| placement == DockPlacement::Bottom)
                    })
                    .map(TabEntry::id)
            });
        Some(selected.map_or(TabFocus::Preserve, TabFocus::Activate))
    }
    pub(in crate::workspace) fn can_split(&self, id: EntityId, cx: &App) -> bool {
        !self.topology.locked(cx)
            && self.item_visible(id, cx)
            && self
                .pane_for_item(id, cx)
                .is_some_and(|pane| pane.panels.len() > 1)
    }
    pub(in crate::workspace) fn split(
        &mut self,
        id: EntityId,
        placement: Placement,
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        if !self.can_split(id, cx) {
            return false;
        }
        let (Some((_, node)), Some(panel)) = (self.item_location(id, cx), self.panel_of(id)) else {
            return false;
        };
        self.groups.leave(panel);
        self.topology.move_panel(
            panel,
            InsertTarget::Split {
                node,
                placement,
                size: None,
            },
            window,
            cx,
        );
        true
    }
    pub(in crate::workspace) fn next_pane(
        &self,
        id: EntityId,
        offset: isize,
        cx: &App,
    ) -> Option<EntityId> {
        let (placement, current) = self.item_location(id, cx)?;
        let panes: Vec<_> = self
            .topology
            .panes(placement, cx)
            .into_iter()
            .filter(|pane| !pane.panels.is_empty())
            .collect();
        let index = panes.iter().position(|pane| pane.node == current)?;
        let target = panes[(index as isize + offset).rem_euclid(panes.len() as isize) as usize]
            .active_panel()?;
        self.item_for_panel(target).map(TabEntry::id)
    }
    pub(in crate::workspace) fn next_tab(
        &self,
        id: EntityId,
        offset: isize,
        cx: &App,
    ) -> Option<EntityId> {
        let pane = self.pane_for_item(id, cx)?;
        let panel = self.panel_of(id)?;
        let index = pane
            .panels
            .iter()
            .position(|candidate| *candidate == panel)?;
        let target =
            pane.panels[(index as isize + offset).rem_euclid(pane.panels.len() as isize) as usize];
        self.item_for_panel(target).map(TabEntry::id)
    }
    pub(in crate::workspace) fn move_tab(
        &self,
        id: EntityId,
        offset: isize,
        window: &mut Window,
        cx: &mut App,
    ) {
        let (Some(pane), Some(panel)) = (self.pane_for_item(id, cx), self.panel_of(id)) else {
            return;
        };
        if self.topology.locked(cx) {
            return;
        }
        let target = pane
            .panels
            .iter()
            .position(|candidate| *candidate == panel)
            .and_then(|index| index.checked_add_signed(offset))
            .filter(|next| *next < pane.panels.len())
            .map(|ix| InsertTarget::Tabs {
                node: pane.node,
                ix: Some(ix),
                activate: true,
            });
        if let Some(target) = target {
            self.topology.move_panel(panel, target, window, cx);
        }
    }
    pub(in crate::workspace) fn tabs_to_close(
        &self,
        id: EntityId,
        scope: TabCloseScope,
        cx: &App,
    ) -> Vec<EntityId> {
        let Some(pane) = self.pane_for_item(id, cx) else {
            return vec![];
        };
        let panels = if scope == TabCloseScope::All {
            self.topology
                .panes(pane.placement, cx)
                .into_iter()
                .flat_map(|pane| pane.panels)
                .collect::<Vec<_>>()
        } else {
            let Some(panel) = self.panel_of(id) else {
                return vec![];
            };
            let Some(clicked) = pane.panels.iter().position(|candidate| *candidate == panel) else {
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
            .filter_map(|panel| self.item_for_panel(panel).map(TabEntry::id))
            .collect()
    }
}
