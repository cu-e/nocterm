//! Hosting, visibility and attention of the independent side panel.

use gpui_kit::{Context, Entity, Subscription, Window};

use super::{Workspace, WorkspaceEvent};
use crate::right_panel::RightPanelHandle;

/// Where the side panel stands. Combinations the flags it replaces allowed,
/// such as a maximized closed panel, cannot be expressed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum RightPanelState {
    /// The host has not offered the panel, or has withdrawn it.
    #[default]
    Unavailable,
    Closed,
    Open {
        maximized: bool,
    },
}

#[derive(Default)]
pub(super) struct RightPanelSlot {
    pub(super) handle: Option<Box<dyn RightPanelHandle>>,
    pub(super) state: RightPanelState,
    /// The panel asks for the user; its toggle shows this whether or not it
    /// is open.
    pub(super) attention: bool,
    subscription: Option<Subscription>,
}

impl RightPanelSlot {
    pub(super) fn available(&self) -> bool {
        self.state != RightPanelState::Unavailable && self.handle.is_some()
    }
    pub(super) fn open(&self) -> bool {
        matches!(self.state, RightPanelState::Open { .. })
    }
    pub(super) fn maximized(&self) -> bool {
        self.state == RightPanelState::Open { maximized: true }
    }
}

impl Workspace {
    pub fn set_right_panel<T: crate::RightPanel>(
        &mut self,
        panel: Entity<T>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.right_panel.subscription = Some(cx.subscribe_in(
            &panel,
            window,
            |this, _, event, window, cx| match event {
                crate::RightPanelEvent::ToggleMaximized => {
                    this.set_right_panel_maximized(!this.right_panel.maximized(), cx)
                }
                crate::RightPanelEvent::Close => this.close_right_panel(window, cx),
                crate::RightPanelEvent::Widen(delta) => this.widen_right_panel(*delta, window, cx),
            },
        ));
        panel.update(cx, |panel, cx| {
            panel.set_docked_left(Self::sides_swapped(cx), cx)
        });
        self.right_panel.handle = Some(Box::new(panel));
        cx.notify();
    }

    pub fn set_right_panel_available(
        &mut self,
        available: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if (self.right_panel.state != RightPanelState::Unavailable) == available {
            return;
        }
        if available {
            self.right_panel.state = RightPanelState::Closed;
        } else {
            self.close_right_panel(window, cx);
            self.right_panel.state = RightPanelState::Unavailable;
        }
        cx.notify();
    }

    pub fn set_right_panel_attention(&mut self, attention: bool, cx: &mut Context<Self>) {
        if self.right_panel.attention != attention {
            self.right_panel.attention = attention;
            cx.notify();
        }
    }
    pub fn right_panel_is_available(&self) -> bool {
        self.right_panel.available()
    }
    pub fn right_panel_is_open(&self) -> bool {
        self.right_panel.open()
    }
    pub fn right_panel_is_maximized(&self) -> bool {
        self.right_panel.maximized()
    }

    pub fn toggle_right_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.right_panel.available() {
            return;
        }
        if self.right_panel.open() {
            self.close_right_panel(window, cx);
        } else {
            self.right_panel.state = RightPanelState::Open { maximized: false };
            cx.emit(WorkspaceEvent::RightPanelVisibilityChanged);
            if let Some(panel) = &self.right_panel.handle {
                window.focus(&panel.focus_handle(cx), cx);
            }
            cx.notify();
        }
    }

    pub fn set_right_panel_maximized(&mut self, maximized: bool, cx: &mut Context<Self>) {
        if !self.right_panel.available()
            || !self.right_panel.open()
            || self.right_panel.maximized() == maximized
        {
            return;
        }
        self.right_panel.state = RightPanelState::Open { maximized };
        if let Some(panel) = &self.right_panel.handle {
            panel.set_maximized(maximized, cx);
        }
        cx.notify();
    }

    pub(super) fn close_right_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let had_focus = self
            .right_panel
            .handle
            .as_ref()
            .is_some_and(|panel| panel.focus_handle(cx).contains_focused(window, cx));
        self.set_right_panel_maximized(false, cx);
        if self.right_panel.open() {
            self.right_panel.state = RightPanelState::Closed;
            cx.emit(WorkspaceEvent::RightPanelVisibilityChanged);
        }
        if had_focus {
            let focus = self
                .active_item()
                .map(|item| item.focus_handle(cx))
                .unwrap_or_else(|| self.focus_handle.clone());
            window.focus(&focus, cx);
        }
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::{RightPanelSlot, RightPanelState};

    #[test]
    fn only_an_open_panel_is_maximized() {
        let mut slot = RightPanelSlot::default();
        assert!(!slot.open() && !slot.maximized() && !slot.available());
        slot.state = RightPanelState::Closed;
        assert!(!slot.open() && !slot.maximized());
        slot.state = RightPanelState::Open { maximized: true };
        assert!(slot.open() && slot.maximized());
    }
}
