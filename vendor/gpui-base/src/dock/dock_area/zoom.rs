// Modified by Nocterm contributors; see NOCTERM.md. Licensed under Apache-2.0.
//! Reconcile versioned group requests without feeding changes back as requests.
use super::*;

impl DockArea {
    /// Zoom a live group if its displayed panel permits it. Explicit dock
    /// commands supersede all group requests that have not been delivered yet.
    pub fn set_zoomed_in(&mut self, node: NodeId, window: &mut Window, cx: &mut Context<Self>) {
        self.override_zoom_requests(Some(Zoomed::Group(node)), window, cx);
    }

    /// Clear both the area's zoom and group flags, including a queued request
    /// whose group changed before the area heard about it.
    pub fn set_zoomed_out(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.override_zoom_requests(None, window, cx);
    }

    pub fn is_zoomed(&self) -> bool {
        self.zoomed.is_some()
    }

    pub fn zoomed_group(&self) -> Option<NodeId> {
        self.zoomed.map(|Zoomed::Group(node)| node)
    }

    fn override_zoom_requests(
        &mut self,
        target: Option<Zoomed>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for (node, cached) in &self.groups {
            cached.entity.update(cx, |group, cx| {
                group.invalidate_zoom_requests();
                if target != Some(Zoomed::Group(*node)) {
                    group.sync_zoomed(false, window, cx);
                }
            });
        }
        self.set_zoom(target, window, cx);
    }

    pub(super) fn reconcile_zoom_request(
        &mut self,
        group: &Entity<TabGroup>,
        revision: u64,
        zoom_in: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let state = group.read(cx);
        let node = state.node();
        if state.zoom_revision() != revision
            || self
                .groups
                .get(&node)
                .is_none_or(|cached| &cached.entity != group)
        {
            return;
        }
        // A different request may have silently cleared this group's flag;
        // its latest pending intent still wins when it reaches the dock.
        if zoom_in {
            self.set_zoom(Some(Zoomed::Group(node)), window, cx);
        } else if self.zoomed == Some(Zoomed::Group(node)) {
            self.set_zoom(None, window, cx);
        }
    }

    /// The sole writer of the area's zoom. Group changes here are silent;
    /// otherwise queued in/out events could perpetually replay one another.
    pub(super) fn set_zoom(
        &mut self,
        target: Option<Zoomed>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(previous) = self.zoomed
            && Some(previous) != target
        {
            self.drive_zoom(previous, false, window, cx);
        }
        let accepted = target.filter(|target| self.drive_zoom(*target, true, window, cx));
        self.zoomed = accepted;
        cx.notify();
    }

    fn drive_zoom(
        &mut self,
        zoomed: Zoomed,
        zoom_in: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Zoomed::Group(node) = zoomed;
        let Some(group) = self.groups.get(&node).map(|cached| cached.entity.clone()) else {
            return false;
        };
        group.update(cx, |group, cx| {
            if zoom_in
                && !group
                    .active_panel(cx)
                    .is_some_and(|panel| panel.zoomable(cx))
            {
                group.sync_zoomed(false, window, cx);
                return false;
            }
            group.sync_zoomed(zoom_in, window, cx);
            group.is_zoomed() == zoom_in
        })
    }

    pub(super) fn zoomed_view(&self) -> Option<AnyView> {
        let Zoomed::Group(node) = self.zoomed?;
        Some(self.groups.get(&node)?.entity.clone().into())
    }
}

#[cfg(test)]
mod tests;
