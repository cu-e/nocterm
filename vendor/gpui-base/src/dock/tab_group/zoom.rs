// Modified by Nocterm contributors; see NOCTERM.md. Licensed under Apache-2.0.
//! User zoom requests and silent dock synchronization have separate lifetimes.
use super::*;

impl TabGroup {
    pub(crate) fn zoom_revision(&self) -> u64 {
        self.zoom_revision
    }

    pub(crate) fn invalidate_zoom_requests(&mut self) {
        self.zoom_revision = self.zoom_revision.wrapping_add(1);
    }

    /// Zoom the group and tell its displayed panel about the accepted change.
    /// Zooming in requires a displayed zoomable panel; zooming out always works.
    /// Only accepted user changes are requests for the containing dock.
    pub(crate) fn set_zoomed(&mut self, zoomed: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !self.sync_zoomed(zoomed, window, cx) {
            return;
        }
        self.invalidate_zoom_requests();
        let revision = self.zoom_revision;
        cx.emit(if zoomed {
            TabGroupEvent::ZoomIn { revision }
        } else {
            TabGroupEvent::ZoomOut { revision }
        });
    }

    /// Dock reconciliation applies the same state/callback contract, without
    /// reporting a new request or invalidating requests already in flight.
    pub(crate) fn sync_zoomed(
        &mut self,
        zoomed: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.zoomed == zoomed {
            return false;
        }
        let panel = self.active_panel(cx);
        if zoomed && !panel.as_ref().is_some_and(|panel| panel.zoomable(cx)) {
            return false;
        }
        self.zoomed = zoomed;
        // A panel may call back into its group, so deliver outside this update.
        if let Some(panel) = panel {
            cx.spawn_in(window, async move |_, cx| {
                _ = cx.update(|window, cx| panel.set_zoomed(zoomed, window, cx));
            })
            .detach();
        }
        cx.notify();
        true
    }
}
