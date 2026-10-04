use gpui_kit::{App, Context, Entity, EventEmitter};

use crate::{Panel, PanelHandle};

/// Requests from an independent right panel to its workspace host.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RightPanelEvent {
    ToggleMaximized,
    Close,
    /// Grow the panel by this much (shrink, if negative), for a column it
    /// opened or closed.
    Widen(gpui_kit::Pixels),
}

/// A panel hosted beside the central dock, including when there are no tabs.
pub trait RightPanel: Panel + EventEmitter<RightPanelEvent> {
    fn set_maximized(&mut self, _maximized: bool, _cx: &mut Context<Self>) {}
    /// The panel moved to the left of the tabs (`true`) or back to the right.
    fn set_docked_left(&mut self, _left: bool, _cx: &mut Context<Self>) {}
}

pub(crate) trait RightPanelHandle: PanelHandle {
    fn set_maximized(&self, maximized: bool, cx: &mut App);
    fn set_docked_left(&self, left: bool, cx: &mut App);
}

impl<T: RightPanel> RightPanelHandle for Entity<T> {
    fn set_maximized(&self, maximized: bool, cx: &mut App) {
        self.update(cx, |panel, cx| panel.set_maximized(maximized, cx));
    }
    fn set_docked_left(&self, left: bool, cx: &mut App) {
        self.update(cx, |panel, cx| panel.set_docked_left(left, cx));
    }
}
