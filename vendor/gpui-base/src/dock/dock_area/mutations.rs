// Modified by Nocterm contributors; see NOCTERM.md. Licensed under Apache-2.0.
//! Accepted topology changes reveal their results; reordering retains zoom.
use super::*;
impl DockArea {
    pub fn move_panel(
        &mut self,
        panel: PanelId,
        target: InsertTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.locked {
            return;
        }
        // A panel this area does not own (e.g. dropped from a nested dock) has
        // no backing entity here; inserting it would strand a ghost tab.
        if self.panel(panel).is_none() {
            return;
        }
        let Some(destination) = self.placement_of_node(target_node(&target)) else {
            return;
        };
        let source = self.placement_of_panel(panel);
        let source_node =
            source.and_then(|placement| self.layout(placement)?.find_panel_node(panel));
        let changes_group = matches!(target, InsertTarget::Split { .. })
            || source_node != Some(target_node(&target));

        // A split target divides an existing slot, so the tree needs real
        // pixels to divide.
        if matches!(target, InsertTarget::Split { .. }) {
            self.adopt_measured_sizes(destination, cx);
        }

        // Read what the source group last told the panel *before* the edit,
        // so the destination can be seeded with it. Without this a panel
        // dragged between groups while displayed is told `true` twice.
        let was_active = self
            .layout(source.unwrap_or(destination))
            .and_then(|tree| tree.find_panel_node(panel))
            .and_then(|node| self.groups.get(&node))
            .and_then(|cached| cached.entity.read(cx).last_notified_active(panel));

        let changed = match source {
            Some(source) if source == destination => {
                let Some(tree) = self.tree_mut(destination) else {
                    return;
                };
                tree.move_panel(panel, target).changed()
            }
            source => {
                // Across trees a move is a detach plus an insert. The detach's
                // `removed_panels` is deliberately dropped on the floor: the
                // panel is still in the dock, so it must not hear `on_removed`.
                //
                // Both halves are committed on, not just the insert. A target
                // whose node kind does not match the insert is a silent no-op
                // in `apply_insert`, and committing on the insert alone would
                // early-return with the panel already gone from the source —
                // stranded in `self.panels`, belonging to no tree, for the
                // next reconcile to prune and destroy.
                let detached = source
                    .and_then(|source| self.tree_mut(source))
                    .is_some_and(|tree| tree.remove_panel(panel).changed());
                let Some(tree) = self.tree_mut(destination) else {
                    return;
                };
                let inserted = tree.insert_panel(panel, target).changed();
                detached || inserted
            }
        };

        if changed && changes_group {
            self.reveal_topology_change(destination, window, cx);
        }
        if changed
            && destination != DockPlacement::Center
            && let Some(pane) = self.docks.get_mut(&destination)
        {
            pane.dock.set_open(true);
        }
        self.commit_changed(changed, window, cx);

        if let Some(active) = was_active {
            if let Some(cached) = self
                .layout(destination)
                .and_then(|tree| tree.find_panel_node(panel))
                .and_then(|node| self.groups.get(&node))
            {
                let group = cached.entity.clone();
                group.update(cx, |group, _| group.seed_active(panel, active));
            }
        }
    }
    /// Register a new panel directly in the requested native group.
    pub fn add_panel_view_to_group(
        &mut self,
        panel: Arc<dyn PanelView>,
        node: NodeId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(placement) = self.placement_of_node(node) else {
            return;
        };
        let id = panel.panel_id(cx);
        self.add_panel_inner(id, panel, (placement, Some(node)), None, window, cx);
    }
    fn reveal_topology_change(
        &mut self,
        placement: DockPlacement,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_zoomed_out(window, cx);
        if let Some(pane) = self.docks.get_mut(&placement) {
            pane.dock.set_open(true);
        }
    }
    pub(super) fn add_panel_inner(
        &mut self,
        id: PanelId,
        panel: Arc<dyn PanelView>,
        destination: (DockPlacement, Option<NodeId>),
        size: Option<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (placement, destination_node) = destination;
        // The registration is written before the target is resolved, because
        // both want `&mut self`, so an add that finds nowhere to put the panel
        // has to undo it. *Undo*, not remove: adding a panel the dock already
        // holds is a legitimate call — a host re-placing one it owns — and
        // dropping its view would strand it in a tree with no entity, which is
        // what `reconcile`'s `views_of` asserts against.
        let previous = self.panels.insert(id, panel);

        // A dock is created to hold the panel; `size` seeds it.
        if placement != DockPlacement::Center && !self.docks.contains_key(&placement) {
            self.docks.insert(
                placement,
                DockRegion {
                    tree: PaneTree::new(RootKind::Any),
                    dock: Dock::new(size.unwrap_or(PANEL_MIN_SIZE * 2.)),
                },
            );
        }

        let Some(tree) = self.tree_mut(placement) else {
            self.restore_registration(id, previous);
            return;
        };
        let target = match destination_node.or_else(|| first_tab_group(tree.root())) {
            Some(node) => InsertTarget::Tabs {
                node,
                ix: None,
                activate: true,
            },
            // An empty region has no container to merge into, so the panel
            // makes one beside the root. `normalize` then removes the emptied
            // root and, for a dock, collapses the wrapper away again.
            None => InsertTarget::Split {
                node: tree.root().id(),
                placement: Placement::Right,
                size,
            },
        };
        let result = tree.insert_panel(id, target);
        if !result.changed() {
            // Nothing took the panel, so a newly registered one must not
            // linger in the view map and be told `on_removed` by the next
            // reconcile.
            self.restore_registration(id, previous);
            return;
        }
        self.commit(result, window, cx);
    }

    /// Put `panel` in a new tab group beside `node`.
    pub fn split_at(
        &mut self,
        node: NodeId,
        panel: PanelId,
        placement: Placement,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.locked {
            return;
        }
        let Some(region) = self.placement_of_node(node) else {
            return;
        };
        self.adopt_measured_sizes(region, cx);
        let Some(tree) = self.tree_mut(region) else {
            return;
        };
        let result = tree.split(node, panel, placement, None);
        if result.changed() {
            self.reveal_topology_change(region, window, cx);
        }
        self.commit(result, window, cx);
    }
}
