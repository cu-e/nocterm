// Modified by Nocterm contributors; see NOCTERM.md. Licensed under Apache-2.0.
//! Native dock visibility preserves pane topology and session ownership.
use super::*;
impl DockArea {
    /// Keep the upstream collapsed strip by default; hosts may supply their own reopen control.
    pub fn set_closed_bottom_strip_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        if self.closed_bottom_strip_visible != visible {
            self.closed_bottom_strip_visible = visible;
            cx.notify();
        }
    }
    /// Toggle the native dock without changing its tree or removing panels.
    pub fn toggle_dock(
        &mut self,
        placement: DockPlacement,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.is_dock_collapsible(placement) && self.is_dock_open(placement) {
            return;
        }
        if self.is_dock_open(placement) {
            self.clear_dock_zoom(placement, window, cx);
        }
        let Some(pane) = self.docks.get_mut(&placement) else {
            return;
        };
        let open = pane.dock.is_open();
        pane.dock.set_open(!open);
        // A closed dock takes its displayed panel off screen, which the
        // active-state contract counts as no panel being displayed — that is
        // what `TabGroupConstraints::collapsed` carries.
        self.reconcile(window, cx);
        cx.emit(DockEvent::LayoutChanged);
    }
    fn clear_dock_zoom(
        &mut self,
        placement: DockPlacement,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for (node, cached) in &self.groups {
            if self.placement_of_node(*node) == Some(placement) {
                cached.entity.update(cx, |group, cx| {
                    group.invalidate_zoom_requests();
                    group.sync_zoomed(false, window, cx);
                });
            }
        }
        if self
            .zoomed_group()
            .is_some_and(|node| self.placement_of_node(node) == Some(placement))
        {
            self.set_zoom(None, window, cx);
        }
    }
    /// Resize one dock from a pointer position, clamped so neither this dock
    /// nor the one opposite is squeezed below its minimum.
    ///
    /// A collapsible bottom dock is the exception: it follows the pointer
    /// below the minimum down to its closed strip, so closing it by drag is
    /// one continuous motion. That size is only shown, and
    /// [`Self::end_dock_resize`] settles it on release.
    pub(super) fn resize_dock(
        &mut self,
        placement: DockPlacement,
        pointer: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let opposite = match placement {
            DockPlacement::Left => self.dock_size(DockPlacement::Right),
            DockPlacement::Right => self.dock_size(DockPlacement::Left),
            _ => None,
        };
        let sizing = DockSizing::new(placement)
            .with_area_bounds(self.bounds)
            .with_opposite_dock_size(opposite.unwrap_or(px(0.)));
        let size = sizing
            .size_from_pointer(pointer)
            .min(sizing.clamp(Pixels::MAX));

        let Some(pane) = self.docks.get_mut(&placement) else {
            return;
        };
        let was_open = pane.dock.is_open();
        if placement == DockPlacement::Bottom && pane.dock.is_collapsible() && size < PANEL_MIN_SIZE
        {
            pane.dock.set_open(size > CLOSED_BOTTOM_STRIP);
            pane.dock.set_live_size(Some(size.max(CLOSED_BOTTOM_STRIP)));
        } else {
            pane.dock.set_open(true);
            pane.dock.set_live_size(None);
            pane.dock.set_size(size);
        }
        if pane.dock.is_open() != was_open {
            if !self.is_dock_open(placement) {
                self.clear_dock_zoom(placement, window, cx);
            }
            self.reconcile(window, cx);
        }
        cx.notify();
    }

    /// Settle a drag that ended below the minimum: nearer the closed strip it
    /// closes, nearer the minimum it opens at the minimum.
    pub(super) fn end_dock_resize(
        &mut self,
        placement: DockPlacement,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(pane) = self.docks.get_mut(&placement) else {
            return;
        };
        let Some(size) = pane.dock.live_size() else {
            return;
        };
        pane.dock.set_live_size(None);

        let open = size >= (CLOSED_BOTTOM_STRIP + PANEL_MIN_SIZE) / 2.;
        if open {
            pane.dock.set_size(PANEL_MIN_SIZE);
        }
        if pane.dock.is_open() != open {
            pane.dock.set_open(open);
            if !open {
                self.clear_dock_zoom(placement, window, cx);
            }
            self.reconcile(window, cx);
        }
        cx.notify();
    }
}
