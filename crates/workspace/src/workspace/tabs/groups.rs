//! Groups of tabs that move together: what the tab menu does with them, and
//! bringing a group back together after the dock moved one of its tabs.
//! The rules live in [`crate::tab_groups`].

use gpui_kit::{
    App, EntityId, SharedString, Window,
    component::dock::{DockPlacement, InsertTarget, NodeId, PanelId},
};

use super::super::docking;
use super::{TabEntry, Tabs};
use crate::{dock_item::GROUP_PALETTE, tab_groups::GroupId};

/// The docks a tab can be dragged to.
const PLACEMENTS: [DockPlacement; 4] = docking::TAB_PLACEMENTS;

impl Tabs {
    /// Puts the tab showing `item` in a group of its own.
    pub(in crate::workspace) fn new_tab_group(
        &mut self,
        item: EntityId,
        window: &mut Window,
        cx: &mut App,
    ) {
        if self.topology.locked(cx) {
            return;
        }
        let Some(panel) = self.panel_of(item) else {
            return;
        };
        self.groups.create(panel, GROUP_PALETTE);
        self.regroup(window, cx);
    }

    /// Puts the tab showing `item` in the group of the tab showing `other`,
    /// starting one when `other` is in none.
    pub(in crate::workspace) fn group_tab_with(
        &mut self,
        item: EntityId,
        other: EntityId,
        window: &mut Window,
        cx: &mut App,
    ) {
        if self.topology.locked(cx) {
            return;
        }
        let (Some(panel), Some(other)) = (self.panel_of(item), self.panel_of(other)) else {
            return;
        };
        if panel == other {
            return;
        }
        let group = self.groups.group_of(other).unwrap_or_else(|| {
            let group = self.groups.create(other, GROUP_PALETTE);
            // Record the target before the newcomer joins, so an existing
            // tab in another placement follows the requested group.
            self.regroup(window, cx);
            group
        });
        self.groups.join(panel, group);
        self.regroup(window, cx);
    }

    /// Takes the tab showing `item` out of its group.
    pub(in crate::workspace) fn ungroup_tab(
        &mut self,
        item: EntityId,
        window: &mut Window,
        cx: &mut App,
    ) {
        if self.topology.locked(cx) {
            return;
        }
        if let Some(panel) = self.panel_of(item) {
            self.groups.leave(panel);
            self.regroup(window, cx);
        }
    }

    /// Ends the group of the tab showing `item`; its tabs stay where they are.
    pub(in crate::workspace) fn dissolve_tab_group(
        &mut self,
        item: EntityId,
        window: &mut Window,
        cx: &mut App,
    ) {
        if self.topology.locked(cx) {
            return;
        }
        if let Some(group) = self.group_of_item(item) {
            self.groups.dissolve(group);
            self.regroup(window, cx);
        }
    }

    /// The items shown in the tabs of `item`'s group, `item` included, in
    /// registration order; empty when it is in none.
    pub(in crate::workspace) fn tab_group_items(&self, item: EntityId) -> Vec<EntityId> {
        let Some(group) = self.group_of_item(item) else {
            return Vec::new();
        };
        self.entries
            .iter()
            .filter(|open| self.groups.group_of(panel(open)) == Some(group))
            .map(|open| open.handle.item_id())
            .collect()
    }

    pub(in crate::workspace) fn tab_group_count(&self) -> usize {
        self.groups.groups().count()
    }

    /// The groups `item` could join, each with the tab showing `beside` it
    /// and a name made of the group's tab titles.
    pub(in crate::workspace) fn tab_groups_to_join(
        &self,
        item: EntityId,
        cx: &App,
    ) -> Vec<(EntityId, SharedString)> {
        let own = self.group_of_item(item);
        self.groups
            .groups()
            .filter(|group| Some(*group) != own)
            .filter_map(|group| {
                let members: Vec<&TabEntry> = self
                    .entries
                    .iter()
                    .filter(|open| self.groups.group_of(panel(open)) == Some(group))
                    .collect();
                let first = members.first()?;
                let title = first
                    .dock_item
                    .read(cx)
                    .alias
                    .clone()
                    .unwrap_or_else(|| first.handle.tab_title(cx));
                let name = match members.len() {
                    1 => title,
                    count => format!("{title} +{}", count - 1).into(),
                };
                Some((first.handle.item_id(), name))
            })
            .collect()
    }

    pub(in crate::workspace) fn is_tab_grouped(&self, item: EntityId) -> bool {
        self.group_of_item(item).is_some()
    }

    /// Brings every group back together after the dock changed, and marks
    /// each tab with its group's color.
    pub(in crate::workspace) fn regroup(&mut self, window: &mut Window, cx: &mut App) {
        let moves = self.groups.gather(&self.tab_bars(cx));
        if !moves.is_empty() {
            for step in moves {
                self.topology.move_panel(
                    step.panel,
                    InsertTarget::Tabs {
                        node: step.node,
                        ix: Some(step.ix),
                        activate: false,
                    },
                    window,
                    cx,
                );
            }
        }
        for open in &self.entries {
            let color = self
                .groups
                .group_of(panel(open))
                .and_then(|group| self.groups.color(group));
            open.dock_item
                .update(cx, |item, cx| item.set_group_color(color, cx));
        }
    }

    /// Joins a tab just opened for a program to the tab it was opened from.
    pub(in crate::workspace) fn group_program(
        &mut self,
        parent: Option<EntityId>,
        opened_from: usize,
        window: &mut Window,
        cx: &mut App,
    ) {
        let opened = self
            .entries
            .get(opened_from..)
            .and_then(|opened| opened.first())
            .map(|open| open.handle.item_id());
        if let (Some(parent), Some(opened)) = (parent, opened) {
            self.group_tab_with(opened, parent, window, cx);
        }
    }

    fn group_of_item(&self, item: EntityId) -> Option<GroupId> {
        self.groups.group_of(self.panel_of(item)?)
    }

    /// Every tab bar a tab can be in, with its tabs from left to right.
    fn tab_bars(&self, cx: &App) -> Vec<(NodeId, Vec<PanelId>)> {
        PLACEMENTS
            .into_iter()
            .flat_map(|placement| self.topology.panes(placement, cx))
            .map(|pane| (pane.node, pane.panels))
            .collect()
    }
}

fn panel(open: &TabEntry) -> PanelId {
    PanelId::from(open.dock_item.entity_id())
}
