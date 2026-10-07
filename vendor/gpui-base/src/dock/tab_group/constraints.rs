// Modified by Nocterm contributors; see NOCTERM.md. Licensed under Apache-2.0.
//! Explicit locks and close guards remain separate from inter-region drag permission.
use super::*;
impl TabGroupConstraints {
    pub fn last_panel_drag_between_regions(mut self, allowed: bool) -> Self {
        self.last_panel_drag_between_regions = allowed;
        self
    }
}
impl TabGroup {
    /// Only explicit dock constraints lock a group; zoom retains tab operations.
    pub(super) fn is_locked(&self) -> bool {
        self.constraints.is_locked()
    }

    /// True when this group holds the last visible panel that anything could
    /// be rearranged around. Only visible panels count, so a hidden panel does
    /// not keep the last visible one draggable and leave the dock empty.
    pub(super) fn is_last_panel(&self, cx: &App) -> bool {
        self.constraints.is_alone() && self.visible_panels(cx).count() <= 1
    }

    pub(super) fn draggable(&self, cx: &App) -> bool {
        !self.is_locked()
            && (!self.is_last_panel(cx) || self.constraints.last_panel_drag_between_regions)
    }

    pub(super) fn droppable(&self) -> bool {
        !self.is_locked()
    }

    pub(super) fn closable_panels(&self, cx: &App) -> bool {
        !self.is_locked() && !self.is_last_panel(cx)
    }
}
