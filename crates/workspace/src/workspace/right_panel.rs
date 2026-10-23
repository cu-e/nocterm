//! Hosting, visibility and attention of the independent side panel.

use gpui_kit::{AnyView, App, FocusHandle, Subscription, Window};

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
    handle: Option<Box<dyn RightPanelHandle>>,
    state: RightPanelState,
    /// The panel asks for the user; its toggle shows this whether or not it
    /// is open.
    attention: bool,
    subscription: Option<Subscription>,
}

pub(super) enum PanelFocus {
    Preserve,
    Panel(FocusHandle),
    Central,
}

pub(super) struct PanelTransition {
    visibility_changed: bool,
    focus: PanelFocus,
}
impl PanelTransition {
    pub(super) fn visibility_changed(&self) -> bool {
        self.visibility_changed
    }
    pub(super) fn into_focus(self) -> PanelFocus {
        self.focus
    }
}

impl RightPanelState {
    fn open(self) -> bool {
        matches!(self, Self::Open { .. })
    }
    fn maximized(self) -> bool {
        self == Self::Open { maximized: true }
    }
}

impl RightPanelSlot {
    pub(super) fn install(
        &mut self,
        handle: Box<dyn RightPanelHandle>,
        subscription: Subscription,
        left: bool,
        cx: &mut App,
    ) {
        handle.set_docked_left(left, cx);
        self.handle = Some(handle);
        self.subscription = Some(subscription);
    }
    pub(super) fn available(&self) -> bool {
        self.state != RightPanelState::Unavailable && self.handle.is_some()
    }
    pub(super) fn open(&self) -> bool {
        self.state.open()
    }
    pub(super) fn maximized(&self) -> bool {
        self.state.maximized()
    }
    pub(super) fn attention(&self) -> bool {
        self.attention
    }
    pub(super) fn focus_handle(&self, cx: &App) -> Option<FocusHandle> {
        self.handle.as_ref().map(|handle| handle.focus_handle(cx))
    }
    pub(super) fn view(&self) -> Option<AnyView> {
        self.handle.as_ref().map(|handle| handle.view())
    }
    pub(super) fn set_docked_left(&self, left: bool, cx: &mut App) {
        if let Some(handle) = &self.handle {
            handle.set_docked_left(left, cx);
        }
    }
    pub(super) fn set_attention(&mut self, attention: bool) -> bool {
        if self.attention == attention {
            return false;
        }
        self.attention = attention;
        true
    }
    pub(super) fn set_maximized(&mut self, maximized: bool, cx: &mut App) -> bool {
        if !self.available() || !self.open() || self.maximized() == maximized {
            return false;
        }
        self.state = RightPanelState::Open { maximized };
        if let Some(handle) = &self.handle {
            handle.set_maximized(maximized, cx);
        }
        true
    }
    pub(super) fn close(&mut self, window: &Window, cx: &mut App) -> PanelTransition {
        let had_focus = self
            .handle
            .as_ref()
            .is_some_and(|panel| panel.focus_handle(cx).contains_focused(window, cx));
        self.set_maximized(false, cx);
        let visibility_changed = self.open();
        if visibility_changed {
            self.state = RightPanelState::Closed;
        }
        PanelTransition {
            visibility_changed,
            focus: if had_focus {
                PanelFocus::Central
            } else {
                PanelFocus::Preserve
            },
        }
    }
    pub(super) fn set_available(
        &mut self,
        available: bool,
        window: &Window,
        cx: &mut App,
    ) -> Option<PanelTransition> {
        if (self.state != RightPanelState::Unavailable) == available {
            return None;
        }
        if available {
            self.state = RightPanelState::Closed;
            Some(PanelTransition {
                visibility_changed: false,
                focus: PanelFocus::Preserve,
            })
        } else {
            let transition = self.close(window, cx);
            self.state = RightPanelState::Unavailable;
            Some(transition)
        }
    }
    pub(super) fn toggle(&mut self, window: &Window, cx: &mut App) -> Option<PanelTransition> {
        if !self.available() {
            return None;
        }
        if self.open() {
            return Some(self.close(window, cx));
        }
        let focus = self.handle.as_ref()?.focus_handle(cx);
        self.state = RightPanelState::Open { maximized: false };
        Some(PanelTransition {
            visibility_changed: true,
            focus: PanelFocus::Panel(focus),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{RightPanelSlot, RightPanelState};
    #[test]
    fn only_an_open_panel_is_maximized() {
        let empty = RightPanelSlot::default();
        assert!(!empty.available() && !empty.open() && !empty.maximized());
        for state in [
            RightPanelState::Unavailable,
            RightPanelState::Closed,
            RightPanelState::Open { maximized: false },
        ] {
            assert!(!state.maximized());
        }
        assert!(!RightPanelState::Unavailable.open());
        assert!(!RightPanelState::Closed.open());
        let state = RightPanelState::Open { maximized: true };
        assert!(state.open() && state.maximized());
    }
}
